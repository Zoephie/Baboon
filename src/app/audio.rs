//! Sound-tag audition: decode-and-play across every game via rodio.
//!
//! Where a `.sound` tag's audio lives depends on the game, and the engine
//! (`blam_tags::audio`) turns each into interleaved PCM:
//! - **CE / H2** — inline in the tag (Ogg Vorbis; Opus / Xbox-ADPCM / PCM).
//! - **Halo 3 / Reach** — FMOD-Vorbis subsounds in `<game>/fmod/pc/*.fsb`
//!   (the tag carries only zeroed placeholder buffers).
//! - **Halo 4** — Wwise: the tag's event name resolves through
//!   `<game>/sound/pc/*.pck` to the media.
//!
//! This app-side layer owns the rodio output device, lazily-opened banks, a
//! decoded-PCM cache, and the pending action the sound-player UI queues (the UI
//! can't touch the output device directly). The Wwise index is large to build,
//! so it loads on a background thread to keep the UI responsive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use blam_tags::audio::{DecodedPcm, SoundBanks, WwiseBanks, decode_subsound, downmix_to_stereo};
use eframe::egui;
use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};

use super::sound_extract::{ExtractRequest, ExtractSource, write_wav_pcm16};

/// Decode a tag-inline classic stream (CE/H2) to interleaved PCM. Shared by the
/// audition (`PlayInline`) and extraction paths.
pub(super) fn decode_inline(
    codec: InlineCodec,
    bytes: &[u8],
    channels: u16,
    sample_rate: u32,
) -> Result<DecodedPcm, String> {
    match codec {
        InlineCodec::OggVorbis => blam_tags::audio::decode_ogg_vorbis(bytes),
        InlineCodec::Opus => blam_tags::audio::decode_opus(bytes, channels),
        InlineCodec::XboxAdpcm => blam_tags::audio::decode_xbox_adpcm(bytes, channels, sample_rate),
        InlineCodec::Pcm { big_endian } => {
            blam_tags::audio::decode_pcm(bytes, channels, sample_rate, big_endian)
        }
    }
}

/// Decode a possibly-chunked H2 inline stream: each chunk (delimited by the
/// `sound_permutation_chunk_block` file offsets) is an independent Opus/ADPCM/PCM
/// stream, so decode each `[offset..next]` slice and concatenate. `chunk_offsets`
/// empty or single = one stream (CE, single-chunk H2).
pub(super) fn decode_inline_chunked(
    codec: InlineCodec,
    bytes: &[u8],
    chunk_offsets: &[usize],
    channels: u16,
    sample_rate: u32,
) -> Result<DecodedPcm, String> {
    if chunk_offsets.len() <= 1 {
        return decode_inline(codec, bytes, channels, sample_rate);
    }
    let mut bounds: Vec<usize> = chunk_offsets.iter().map(|&o| o.min(bytes.len())).collect();
    bounds.push(bytes.len());
    let mut acc: Option<DecodedPcm> = None;
    for w in bounds.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        if let Ok(pcm) = decode_inline(codec, &bytes[a..b], channels, sample_rate) {
            match &mut acc {
                None => acc = Some(pcm),
                Some(x) => x.samples.extend_from_slice(&pcm.samples),
            }
        }
        // A bad chunk is skipped; the rest still decode.
    }
    acc.ok_or_else(|| "no decodable chunks".to_owned())
}

/// Why an FMOD bank resolve/decode failed, so callers can phrase it for play
/// (per-permutation) or extract (per-file).
enum BankError {
    NoBank,
    NotFound,
    Decode(String),
}

/// An audition action queued by the sound-player UI, drained each frame by
/// [`AudioState::process`].
/// A codec for tag-inline audio (classic CE/H2). Ogg Vorbis is self-describing;
/// Opus/Xbox-ADPCM need the channel count (and ADPCM the sample rate) supplied
/// from the tag, since their raw streams don't carry it.
#[derive(Clone, Copy, Debug)]
pub(super) enum InlineCodec {
    OggVorbis,
    Opus,
    XboxAdpcm,
    /// Uncompressed interleaved 16-bit PCM (H2 "none" compression).
    Pcm {
        big_endian: bool,
    },
}

pub(super) enum SoundAction {
    /// Play the FMOD bank subsound named `key` (a permutation string-id).
    /// Used by Halo 3+ whose audio is paged out to `<game>/fmod/pc/*.fsb`.
    Play { key: String, label: String },
    /// Play encoded audio stored *inline* in the tag (classic Halo CE/H2).
    /// `chunk_offsets` are H2 per-chunk byte offsets into `bytes` (each chunk is
    /// an independent stream, concatenated on decode); empty = one stream (CE).
    PlayInline {
        bytes: Vec<u8>,
        codec: InlineCodec,
        channels: u16,
        sample_rate: u32,
        chunk_offsets: Vec<usize>,
        label: String,
    },
    /// Play a Wwise event by name (Halo 4). The audio lives in
    /// `<game>/sound/pc/*.pck`; the tag only carries the event name.
    PlayEvent { event_name: String, label: String },
    /// Set the playback volume (linear amplitude, 0.0..=1.0). Applies to every
    /// live voice immediately and to all subsequent plays.
    SetVolume(f32),
    /// Select the localized language (`None` = default) for bank/pck resolution.
    /// Re-opens the banks on the next play/extract.
    SetLanguage(Option<String>),
    /// Stop everything currently playing.
    Stop,
}

/// Linear playback volume (amplitude multiplier). Wrapped so [`AudioState`] can
/// keep `#[derive(Default)]` while defaulting to full volume, not silence.
#[derive(Clone, Copy)]
pub(super) struct Volume(f32);

impl Default for Volume {
    fn default() -> Self {
        Self(1.0)
    }
}

/// The rodio output device + its live voices. Field order matters: the sinks
/// must drop before the stream.
struct Engine {
    voices: Vec<Sink>,
    /// Applied to every new voice, so playback honours the current volume.
    volume: f32,
    handle: OutputStreamHandle,
    _stream: OutputStream,
}

impl Engine {
    fn new(volume: f32) -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Some(Self {
                voices: Vec::new(),
                volume,
                handle,
                _stream: stream,
            }),
            Err(_) => None,
        }
    }

    /// Update the volume and apply it to everything currently playing.
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        for voice in &self.voices {
            voice.set_volume(volume);
        }
    }

    fn play(&mut self, pcm: &DecodedPcm) {
        // Fold >2 channels down to stereo for the output device.
        let (samples, channels) = if pcm.channels > 2 {
            (downmix_to_stereo(&pcm.samples, pcm.channels as usize), 2u16)
        } else {
            (pcm.samples.clone(), pcm.channels)
        };
        if samples.is_empty() {
            return;
        }
        let Ok(sink) = Sink::try_new(&self.handle) else {
            return;
        };
        sink.set_volume(self.volume);
        let source = SamplesBuffer::new(channels, pcm.sample_rate, samples);
        sink.append(source.convert_samples::<f32>());
        self.voices.push(sink);
    }

    fn stop_all(&mut self) {
        for voice in self.voices.drain(..) {
            voice.stop();
        }
    }

    /// Drop finished voices so the pool doesn't grow unbounded.
    fn reap(&mut self) {
        self.voices.retain(|voice| !voice.empty());
    }
}

/// App-owned audio state. Everything is lazy: the output device opens on the
/// first play, the banks open on the first resolve for a given source.
#[derive(Default)]
pub(super) struct AudioState {
    engine: Option<Engine>,
    engine_tried: bool,
    banks: Option<SoundBanks>,
    banks_root: Option<PathBuf>,
    /// The language the currently-open FMOD banks were opened for (so a language
    /// change re-opens them). `Some(None)` = the default/primary set.
    banks_lang: Option<Option<String>>,
    cache: HashMap<(usize, usize), Arc<DecodedPcm>>,
    /// Lazily-opened Wwise packages (Halo 4) + a decoded-event cache. The index
    /// is built on a background thread (`wwise_loading`), since it reads every
    /// bank; `wwise_root` marks which source it belongs to. `None` after a load
    /// that found no packages.
    wwise: Option<WwiseBanks>,
    wwise_root: Option<PathBuf>,
    /// The dialogue language the current Wwise index was built for.
    wwise_lang: Option<String>,
    /// In-flight background index build: the source root + language it's for, and
    /// the channel it will deliver the opened banks (or `None`) on.
    wwise_loading: Option<(PathBuf, Option<String>, Receiver<Option<WwiseBanks>>)>,
    /// An event queued to play as soon as the in-flight load finishes.
    wwise_deferred: Option<(String, String)>,
    event_cache: HashMap<String, Arc<DecodedPcm>>,
    /// Current playback volume (linear, 0.0..=1.0). Held here so it survives
    /// before the engine is lazily created and seeds it on first play.
    volume: Volume,
    /// Selected localized language (`None` = default/primary), applied when
    /// opening FMOD/Wwise banks so audition + extraction use that language.
    /// `pub(super)` for a field-disjoint borrow at `FieldEditContext` build sites.
    pub(super) language: Option<String>,
    /// Set by the sound-player UI; drained by [`AudioState::process`].
    pub(super) pending: Option<SoundAction>,
    /// Last user-facing status line (bank/resolve/playback result).
    pub(super) status: Option<String>,
}

impl AudioState {
    /// Lazily open the FMOD banks under `<game>/fmod/pc/` for this source +
    /// selected language, re-opening when either changes.
    fn ensure_banks(&mut self, tags_root: &Path) -> Option<&SoundBanks> {
        let lang = self.language.clone();
        if self.banks_root.as_deref() != Some(tags_root) || self.banks_lang.as_ref() != Some(&lang)
        {
            self.banks = SoundBanks::open_pc_language(tags_root, lang.as_deref()).ok();
            self.banks_root = Some(tags_root.to_path_buf());
            self.banks_lang = Some(lang);
            self.cache.clear();
        }
        self.banks.as_ref()
    }

    /// Resolve an FMOD permutation `key` to a decoded (cached) buffer. Shared by
    /// audition and extraction; the bank borrow is scoped so it drops before the
    /// cache insert.
    fn bank_pcm(&mut self, tags_root: &Path, key: &str) -> Result<Arc<DecodedPcm>, BankError> {
        let resolved = match self.ensure_banks(tags_root) {
            Some(banks) => banks.resolve(key),
            None => return Err(BankError::NoBank),
        };
        let Some((bank_index, sub_index)) = resolved else {
            return Err(BankError::NotFound);
        };
        if let Some(pcm) = self.cache.get(&(bank_index, sub_index)) {
            return Ok(pcm.clone());
        }
        let decoded = {
            let banks = self.banks.as_ref().expect("banks opened above");
            let bank = banks.bank(bank_index);
            let sub = &bank.subsounds[sub_index];
            match bank.read_subsound_data(sub_index) {
                Ok(data) => decode_subsound(&data, sub.channels, sub.frequency, sub.setup_hash),
                Err(err) => Err(err),
            }
        };
        match decoded {
            Ok(pcm) => {
                let pcm = Arc::new(pcm);
                self.cache.insert((bank_index, sub_index), pcm.clone());
                Ok(pcm)
            }
            Err(err) => Err(BankError::Decode(err)),
        }
    }

    /// Drain a queued extraction batch: decode each item (reusing the audition
    /// decoders/banks) and write it to disk, then report how many landed. Runs
    /// synchronously — batches are a handful of permutations.
    pub(super) fn run_extract(&mut self, request: ExtractRequest) {
        let total = request.items.len();
        let mut ok = 0usize;
        let mut first_err: Option<String> = None;
        for item in request.items {
            let result: Result<(), String> = match item.source {
                ExtractSource::Raw(bytes) => {
                    if let Some(parent) = item.out_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::write(&item.out_path, &bytes).map_err(|e| e.to_string())
                }
                ExtractSource::Inline {
                    bytes,
                    codec,
                    channels,
                    sample_rate,
                    chunk_offsets,
                } => decode_inline_chunked(codec, &bytes, &chunk_offsets, channels, sample_rate)
                    .and_then(|pcm| {
                        write_wav_pcm16(&item.out_path, &pcm.samples, pcm.channels, pcm.sample_rate)
                            .map_err(|e| e.to_string())
                    }),
                ExtractSource::Bank { key } => match request.tags_root.as_deref() {
                    None => Err("no source loaded".to_owned()),
                    Some(tags_root) => match self.bank_pcm(tags_root, &key) {
                        Ok(pcm) => write_wav_pcm16(
                            &item.out_path,
                            &pcm.samples,
                            pcm.channels,
                            pcm.sample_rate,
                        )
                        .map_err(|e| e.to_string()),
                        Err(BankError::NoBank) => Err("no FMOD bank".to_owned()),
                        Err(BankError::NotFound) => Err(format!("'{key}' not in bank")),
                        Err(BankError::Decode(e)) => Err(e),
                    },
                },
                ExtractSource::Event { name } => match self.wwise.as_ref() {
                    Some(banks) => banks.resolve(&name).and_then(|pcm| {
                        write_wav_pcm16(&item.out_path, &pcm.samples, pcm.channels, pcm.sample_rate)
                            .map_err(|e| e.to_string())
                    }),
                    None => Err("play the event first to load Wwise banks".to_owned()),
                },
            };
            match result {
                Ok(()) => ok += 1,
                Err(err) => {
                    if first_err.is_none() {
                        first_err = Some(err);
                    }
                }
            }
        }
        self.status = Some(match first_err {
            None => format!("extracted {ok}/{total} \u{2014} {}", request.label),
            Some(err) => format!("extracted {ok}/{total} \u{2014} {} ({err})", request.label),
        });
    }

    /// Kick off a background build of the Wwise index for `tags_root` (unless
    /// one is already in flight for the same root). Reads every bank to build
    /// the event graph, so it must not run on the UI thread. `ctx` is pinged
    /// when it finishes so the drain loop picks up the result promptly.
    fn start_wwise_load(&mut self, tags_root: &Path, ctx: &egui::Context) {
        let lang = self.language.clone();
        if let Some((r, l, _)) = self.wwise_loading.as_ref() {
            if r.as_path() == tags_root && l.as_deref() == lang.as_deref() {
                return; // already loading this root + language
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let root = tags_root.to_path_buf();
        let thread_root = root.clone();
        let thread_lang = lang.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let banks = WwiseBanks::open_pc_language(&thread_root, thread_lang.as_deref()).ok();
            let _ = tx.send(banks);
            ctx.request_repaint();
        });
        self.wwise_loading = Some((root, lang, rx));
    }

    /// Poll the in-flight Wwise load; on completion, store the banks and play
    /// any event that was deferred while it built. Returns early (borrow
    /// released) while the load is still running.
    fn poll_wwise_load(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let banks = match self.wwise_loading.as_ref() {
            Some((_, _, rx)) => match rx.try_recv() {
                Ok(banks) => banks,                      // finished (Some/None banks)
                Err(TryRecvError::Empty) => return,      // still loading
                Err(TryRecvError::Disconnected) => None, // worker died
            },
            None => return,
        };
        let (root, lang) = self
            .wwise_loading
            .take()
            .map(|(r, l, _)| (Some(r), l))
            .unwrap_or((None, None));
        let ok = banks.is_some();
        self.wwise = banks;
        self.wwise_root = root;
        self.wwise_lang = lang;
        self.event_cache.clear();
        match self.wwise_deferred.take() {
            Some((event_name, label)) => self.play_event(&event_name, &label),
            None if !ok => self.status = Some("no Wwise .pck under <game>/sound/pc".to_owned()),
            None => {}
        }
    }

    /// Resolve an event name to PCM (cached) and play it. Assumes the banks for
    /// the current source are already loaded (`wwise_root` set).
    fn play_event(&mut self, event_name: &str, label: &str) {
        if let Some(pcm) = self.event_cache.get(event_name).cloned() {
            self.play_decoded(&pcm, label);
            return;
        }
        let decoded = match self.wwise.as_ref() {
            Some(banks) => banks.resolve(event_name),
            None => Err("no Wwise .pck under <game>/sound/pc".to_owned()),
        };
        match decoded {
            Ok(pcm) => {
                let pcm = Arc::new(pcm);
                self.event_cache.insert(event_name.to_owned(), pcm.clone());
                self.play_decoded(&pcm, label);
            }
            Err(err) => self.status = Some(format!("resolve failed: {err}")),
        }
    }

    /// True while a background Wwise index build is in flight (the caller should
    /// keep requesting repaints so the drain loop polls it).
    pub(super) fn is_busy(&self) -> bool {
        self.wwise_loading.is_some()
    }

    /// The current playback volume (linear, 0.0..=1.0), for the UI slider.
    pub(super) fn volume(&self) -> f32 {
        self.volume.0
    }

    fn ensure_engine(&mut self) -> Option<&mut Engine> {
        if !self.engine_tried {
            self.engine = Engine::new(self.volume.0);
            self.engine_tried = true;
        }
        self.engine.as_mut()
    }

    /// Drain the pending UI action: resolve the subsound, decode (cached), play.
    pub(super) fn process(&mut self, tags_root: Option<&Path>, ctx: &egui::Context) {
        if let Some(engine) = self.engine.as_mut() {
            engine.reap();
        }
        // Pick up a finished background Wwise load (and play any deferred event).
        self.poll_wwise_load();
        let Some(action) = self.pending.take() else {
            return;
        };
        let (key, label) = match action {
            SoundAction::SetVolume(v) => {
                let v = v.clamp(0.0, 1.0);
                self.volume = Volume(v);
                if let Some(engine) = self.engine.as_mut() {
                    engine.set_volume(v);
                }
                return;
            }
            SoundAction::SetLanguage(lang) => {
                if self.language != lang {
                    self.language = lang;
                    // FMOD banks re-open lazily (ensure_banks checks the language);
                    // drop the Wwise index so the next event reloads its language.
                    self.wwise = None;
                    self.wwise_root = None;
                    self.wwise_lang = None;
                    self.event_cache.clear();
                }
                return;
            }
            SoundAction::Stop => {
                if let Some(engine) = self.engine.as_mut() {
                    engine.stop_all();
                }
                self.wwise_deferred = None; // cancel a play waiting on a load
                self.status = Some("stopped".to_owned());
                return;
            }
            SoundAction::PlayInline {
                bytes,
                codec,
                channels,
                sample_rate,
                chunk_offsets,
                label,
            } => {
                self.wwise_deferred = None; // superseded by this playback
                // Classic CE/H2: audio is inline in the tag (H2 is chunked).
                match decode_inline_chunked(codec, &bytes, &chunk_offsets, channels, sample_rate) {
                    Ok(pcm) => self.play_decoded(&pcm, &label),
                    Err(err) => self.status = Some(format!("decode failed: {err}")),
                }
                return;
            }
            SoundAction::PlayEvent { event_name, label } => {
                let Some(tags_root) = tags_root else {
                    self.status = Some("no source loaded".to_owned());
                    return;
                };
                // Banks already built for this source + language? Resolve now.
                if self.wwise_root.as_deref() == Some(tags_root) && self.wwise_lang == self.language
                {
                    self.play_event(&event_name, &label);
                } else {
                    // First event for this source: build the index off-thread
                    // (it reads every bank) and play once it's ready.
                    self.start_wwise_load(tags_root, ctx);
                    self.wwise_deferred = Some((event_name, label));
                    self.status = Some("loading sound banks\u{2026}".to_owned());
                }
                return;
            }
            SoundAction::Play { key, label } => (key, label),
        };
        self.wwise_deferred = None; // FMOD playback supersedes a pending event

        let Some(tags_root) = tags_root else {
            self.status = Some("no source loaded".to_owned());
            return;
        };

        match self.bank_pcm(tags_root, &key) {
            Ok(pcm) => self.play_decoded(&pcm, &label),
            Err(BankError::NoBank) => {
                self.status = Some("no FMOD bank under <game>/fmod/pc".to_owned())
            }
            Err(BankError::NotFound) => {
                self.status = Some(format!("'{label}' not found in FMOD bank"))
            }
            Err(BankError::Decode(err)) => self.status = Some(format!("decode failed: {err}")),
        }
    }

    /// Play an already-decoded buffer on a fresh voice (stopping others).
    fn play_decoded(&mut self, pcm: &DecodedPcm, label: &str) {
        let secs = pcm.duration_secs();
        match self.ensure_engine() {
            Some(engine) => {
                engine.stop_all();
                engine.play(pcm);
                self.status = Some(format!("\u{25B6} {label}  ({secs:.2}s)"));
            }
            None => self.status = Some("no audio output device".to_owned()),
        }
    }
}
