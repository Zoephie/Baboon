//! Sound-tag audition: decode-and-play across every game via rodio.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.
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

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, PoisonError};

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
    /// Play an FMOD bank subsound (Halo 3+, audio paged out to
    /// `<game>/fmod/pc/*.fsb`). `id` is the engine's `fmod bank subsound id
    /// hash` (preferred, collision-free); `key` is the permutation leaf name
    /// (legacy fallback for banks without a `.fsb.info` manifest).
    Play {
        id: Option<u32>,
        key: String,
        label: String,
    },
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
    /// Play one Campaign Evolved Wwise media file. Unlike Halo 4 the tag names
    /// no event, so the media is resolved up front by walking package imports
    /// (see [`crate::source::ce_audio`]) and the already-resolved entry is
    /// handed here. `paks_root` is the source's `Paks` directory, which holds
    /// the legacy `.pak` containers the media is staged in.
    PlayCeMedia {
        paks_root: std::path::PathBuf,
        media: Box<crate::source::ce_audio::CeSoundMedia>,
        label: String,
    },
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
    banks: Option<Arc<SoundBanks>>,
    /// Bumped whenever `banks` is reopened, so a decode that finishes for the
    /// old banks is not cached under indices into the new ones.
    banks_generation: u64,
    banks_root: Option<PathBuf>,
    /// The language the currently-open FMOD banks were opened for (so a language
    /// change re-opens them). `Some(None)` = the default/primary set.
    banks_lang: Option<Option<String>>,
    cache: PcmCache<(usize, usize)>,
    /// Lazily-opened Wwise packages (Halo 4) + a decoded-event cache. The index
    /// is built on a background thread (`wwise_loading`), since it reads every
    /// bank; `wwise_root` marks which source it belongs to. `None` after a load
    /// that found no packages.
    wwise: Option<Arc<WwiseBanks>>,
    /// Bumped whenever `wwise` changes; see `banks_generation`.
    wwise_generation: u64,
    wwise_root: Option<PathBuf>,
    /// The dialogue language the current Wwise index was built for.
    wwise_lang: Option<String>,
    /// In-flight background index build: the source root + language it's for, and
    /// the channel it will deliver the opened banks (or `None`) on.
    wwise_loading: Option<(PathBuf, Option<String>, Receiver<Option<WwiseBanks>>)>,
    /// An event queued to play as soon as the in-flight load finishes.
    wwise_deferred: Option<(String, String)>,
    event_cache: PcmCache<String>,
    /// Campaign Evolved's legacy `.pak` set, opened on first playback. CE media
    /// is not in IoStore, so this is a separate store from `wwise` above.
    /// `pub(super)` because binding resolution also needs it: an event whose
    /// media lives inside a SoundBank is only readable through the pak set.
    ///
    /// Shared with the decode workers, which hold the lock only to fetch a
    /// media file's bytes and decode after releasing it.
    pub(super) ce_media: Arc<Mutex<crate::source::ce_audio::CeMediaStore>>,
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
    /// Decodes and extraction batches running on workers report back here.
    jobs: AudioJobs,
    /// The playback most recently asked for. A decode finishing for an older
    /// one is cached but not played: the user has moved on, or pressed Stop.
    play_request: u64,
}

/// Decoded audio kept for replay, bounded by size. Once over the budget the
/// least recently played entries go first.
struct PcmCache<K> {
    map: HashMap<K, Arc<DecodedPcm>>,
    order: VecDeque<K>,
    bytes: usize,
    budget: usize,
}

/// Per cache. A minute of 48 kHz stereo is ~11 MiB of samples.
const PCM_CACHE_BUDGET: usize = 128 << 20;

impl<K> Default for PcmCache<K> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            budget: PCM_CACHE_BUDGET,
        }
    }
}

fn pcm_bytes(pcm: &DecodedPcm) -> usize {
    pcm.samples.len() * std::mem::size_of::<i16>()
}

impl<K: Hash + Eq + Clone> PcmCache<K> {
    fn get(&mut self, key: &K) -> Option<Arc<DecodedPcm>> {
        let pcm = self.map.get(key)?.clone();
        if let Some(position) = self.order.iter().position(|entry| entry == key)
            && let Some(entry) = self.order.remove(position)
        {
            self.order.push_back(entry);
        }
        Some(pcm)
    }

    fn insert(&mut self, key: K, pcm: Arc<DecodedPcm>) {
        if let Some(old) = self.map.insert(key.clone(), pcm.clone()) {
            self.bytes -= pcm_bytes(&old);
            self.order.retain(|entry| entry != &key);
        }
        self.bytes += pcm_bytes(&pcm);
        self.order.push_back(key);
        // The newest entry stays even when it alone is over budget: it is
        // the one about to be replayed.
        while self.bytes > self.budget && self.order.len() > 1 {
            if let Some(evicted) = self.order.pop_front()
                && let Some(old) = self.map.remove(&evicted)
            {
                self.bytes -= pcm_bytes(&old);
            }
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.bytes = 0;
    }
}

/// Where a finished decode belongs in the caches, stamped with the bank
/// generation it was decoded against.
enum PcmKey {
    Bank { generation: u64, bank: usize, sub: usize },
    Event { generation: u64, name: String },
}

/// A worker's report.
enum AudioDone {
    Decoded {
        request: u64,
        cache: Option<PcmKey>,
        label: String,
        result: Result<Arc<DecodedPcm>, String>,
    },
    Extracted(String),
}

struct AudioJobs {
    tx: Sender<AudioDone>,
    rx: Receiver<AudioDone>,
    running: usize,
}

impl Default for AudioJobs {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { tx, rx, running: 0 }
    }
}

/// Fetch a Campaign Evolved media file under the store's lock, then decode it
/// without holding it.
fn decode_ce_media(
    store: &Mutex<crate::source::ce_audio::CeMediaStore>,
    paks_root: &Path,
    media: &crate::source::ce_audio::CeSoundMedia,
) -> Result<DecodedPcm, String> {
    let bytes = store
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fetch(paks_root, media)
        .map_err(|error| format!("{error:#}"))?;
    crate::source::ce_audio::decode_media_bytes(media, &bytes).map_err(|error| format!("{error:#}"))
}

fn decode_bank_subsound(
    banks: &SoundBanks,
    bank_index: usize,
    sub_index: usize,
) -> Result<DecodedPcm, String> {
    let bank = banks.bank(bank_index);
    let sub = &bank.subsounds[sub_index];
    let data = bank.read_subsound_data(sub_index)?;
    decode_subsound(&data, sub.channels, sub.frequency, sub.setup_hash)
}

/// What an extraction batch reads from, captured on the UI thread.
struct ExtractSources {
    banks: Option<Arc<SoundBanks>>,
    wwise: Option<Arc<WwiseBanks>>,
    ce_media: Arc<Mutex<crate::source::ce_audio::CeMediaStore>>,
}

/// Decode and write every item in a batch. Runs on a worker; returns the
/// status line.
fn extract_batch(request: ExtractRequest, sources: &ExtractSources) -> String {
    let total = request.items.len();
    let mut ok = 0usize;
    let mut first_err: Option<String> = None;
    let write = |path: &Path, pcm: &DecodedPcm| {
        write_wav_pcm16(path, &pcm.samples, pcm.channels, pcm.sample_rate)
            .map_err(|e| e.to_string())
    };
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
                .and_then(|pcm| write(&item.out_path, &pcm)),
            ExtractSource::Bank { id, key } => match sources.banks.as_deref() {
                None => Err("no FMOD bank".to_owned()),
                Some(banks) => match resolve_bank(banks, id, &key) {
                    None => Err(format!("'{key}' not in bank")),
                    Some((bank, sub)) => decode_bank_subsound(banks, bank, sub)
                        .and_then(|pcm| write(&item.out_path, &pcm)),
                },
            },
            ExtractSource::CeMedia { paks_root, media } => {
                decode_ce_media(&sources.ce_media, &paks_root, &media)
                    .and_then(|pcm| write(&item.out_path, &pcm))
            }
            ExtractSource::Event { name } => match sources.wwise.as_deref() {
                Some(banks) => banks
                    .resolve(&name)
                    .and_then(|pcm| write(&item.out_path, &pcm)),
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
    match first_err {
        None => format!("extracted {ok}/{total} \u{2014} {}", request.label),
        Some(err) => format!("extracted {ok}/{total} \u{2014} {} ({err})", request.label),
    }
}

/// Prefers the engine's `fmod bank subsound id hash` (`id`), which uniquely
/// identifies the intended permutation across the whole bank. Falls back to
/// the legacy `key` (permutation leaf name) when the id isn't available or
/// the bank has no `.fsb.info` — the name lookup is ambiguous (many tags
/// share a leaf) but is all older/non-MCC banks offer.
fn resolve_bank(banks: &SoundBanks, id: Option<u32>, key: &str) -> Option<(usize, usize)> {
    id.and_then(|id| banks.resolve_by_id(id))
        .or_else(|| banks.resolve(key))
}

impl AudioState {
    /// Lazily open the FMOD banks under `<game>/fmod/pc/` for this source +
    /// selected language, re-opening when either changes.
    fn ensure_banks(&mut self, tags_root: &Path) -> Option<&SoundBanks> {
        let lang = self.language.clone();
        if self.banks_root.as_deref() != Some(tags_root) || self.banks_lang.as_ref() != Some(&lang)
        {
            self.banks = SoundBanks::open_pc_language(tags_root, lang.as_deref())
                .ok()
                .map(Arc::new);
            self.banks_root = Some(tags_root.to_path_buf());
            self.banks_lang = Some(lang);
            self.banks_generation += 1;
            self.cache.clear();
        }
        self.banks.as_deref()
    }

    /// Start a decode on a worker. Whatever finishes is cached under `cache`
    /// if its banks are still the current ones, and played if it is still the
    /// newest playback asked for.
    fn spawn_decode(
        &mut self,
        label: String,
        cache: Option<PcmKey>,
        ctx: &egui::Context,
        decode: impl FnOnce() -> Result<DecodedPcm, String> + Send + 'static,
    ) {
        self.play_request += 1;
        let request = self.play_request;
        self.status = Some(format!("decoding {label}\u{2026}"));
        self.spawn_job(ctx, move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode))
                .unwrap_or_else(|panic| Err(super::state::panic_text(&panic)))
                .map(Arc::new);
            AudioDone::Decoded {
                request,
                cache,
                label,
                result,
            }
        });
    }

    fn spawn_job(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce() -> AudioDone + Send + 'static,
    ) {
        self.jobs.running += 1;
        let tx = self.jobs.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }

    /// Apply what the workers have finished.
    fn drain_jobs(&mut self) {
        while let Ok(done) = self.jobs.rx.try_recv() {
            self.apply_job(done);
        }
    }

    fn apply_job(&mut self, done: AudioDone) {
        self.jobs.running = self.jobs.running.saturating_sub(1);
        match done {
            AudioDone::Extracted(status) => self.status = Some(status),
            AudioDone::Decoded {
                request,
                cache,
                label,
                result,
            } => {
                let pcm = match result {
                    Ok(pcm) => pcm,
                    Err(error) => {
                        if request == self.play_request {
                            self.status = Some(format!("decode failed: {error}"));
                        }
                        return;
                    }
                };
                match cache {
                    Some(PcmKey::Bank {
                        generation,
                        bank,
                        sub,
                    }) if generation == self.banks_generation => {
                        self.cache.insert((bank, sub), pcm.clone());
                    }
                    Some(PcmKey::Event { generation, name })
                        if generation == self.wwise_generation =>
                    {
                        self.event_cache.insert(name, pcm.clone());
                    }
                    _ => {}
                }
                if request == self.play_request {
                    self.play_decoded(&pcm, &label);
                }
            }
        }
    }

    /// Block until every running job has reported, applying each.
    #[cfg(test)]
    pub(super) fn wait_for_audio_jobs(&mut self) {
        while self.jobs.running > 0 {
            match self.jobs.rx.recv() {
                Ok(done) => self.apply_job(done),
                Err(_) => break,
            }
        }
    }

    /// Queue an extraction batch on a worker: decode each item (with the
    /// audition decoders and banks) and write it to disk. Dialogue tags carry
    /// dozens of permutations, which is too long to hold a frame for.
    pub(super) fn run_extract(&mut self, request: ExtractRequest, ctx: &egui::Context) {
        let has_bank_items = request
            .items
            .iter()
            .any(|item| matches!(item.source, ExtractSource::Bank { .. }));
        if has_bank_items && let Some(tags_root) = request.tags_root.as_deref() {
            self.ensure_banks(tags_root);
        }
        let sources = ExtractSources {
            banks: self.banks.clone(),
            wwise: self.wwise.clone(),
            ce_media: self.ce_media.clone(),
        };
        self.status = Some(format!("extracting {}\u{2026}", request.label));
        self.spawn_job(ctx, move || {
            AudioDone::Extracted(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    extract_batch(request, &sources)
                }))
                .unwrap_or_else(|panic| {
                    format!("extraction failed: {}", super::state::panic_text(&panic))
                }),
            )
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
    fn poll_wwise_load(&mut self, ctx: &egui::Context) {
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
        self.wwise = banks.map(Arc::new);
        self.wwise_generation += 1;
        self.wwise_root = root;
        self.wwise_lang = lang;
        self.event_cache.clear();
        match self.wwise_deferred.take() {
            Some((event_name, label)) => self.play_event(&event_name, &label, ctx),
            None if !ok => self.status = Some("no Wwise .pck under <game>/sound/pc".to_owned()),
            None => {}
        }
    }

    /// Resolve an event name to PCM (cached) and play it. Assumes the banks for
    /// the current source are already loaded (`wwise_root` set).
    fn play_event(&mut self, event_name: &str, label: &str, ctx: &egui::Context) {
        if let Some(pcm) = self.event_cache.get(&event_name.to_owned()) {
            self.play_request += 1;
            self.play_decoded(&pcm, label);
            return;
        }
        let Some(banks) = self.wwise.clone() else {
            self.status = Some("resolve failed: no Wwise .pck under <game>/sound/pc".to_owned());
            return;
        };
        let name = event_name.to_owned();
        let cache = PcmKey::Event {
            generation: self.wwise_generation,
            name: name.clone(),
        };
        self.spawn_decode(label.to_owned(), Some(cache), ctx, move || {
            banks
                .resolve(&name)
                .map_err(|error| format!("resolve failed: {error}"))
        });
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
        self.drain_jobs();
        // Pick up a finished background Wwise load (and play any deferred event).
        self.poll_wwise_load(ctx);
        let Some(action) = self.pending.take() else {
            return;
        };
        let (id, key, label) = match action {
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
                    self.wwise_generation += 1;
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
                self.play_request += 1; // and a decode still running
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
                self.spawn_decode(label, None, ctx, move || {
                    decode_inline_chunked(codec, &bytes, &chunk_offsets, channels, sample_rate)
                });
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
                    self.play_event(&event_name, &label, ctx);
                } else {
                    // First event for this source: build the index off-thread
                    // (it reads every bank) and play once it's ready.
                    self.start_wwise_load(tags_root, ctx);
                    self.wwise_deferred = Some((event_name, label));
                    self.status = Some("loading sound banks\u{2026}".to_owned());
                }
                return;
            }
            SoundAction::PlayCeMedia {
                paks_root,
                media,
                label,
            } => {
                self.wwise_deferred = None; // this playback supersedes any wait
                let store = self.ce_media.clone();
                self.spawn_decode(label, None, ctx, move || {
                    decode_ce_media(&store, &paks_root, &media)
                });
                return;
            }
            SoundAction::Play { id, key, label } => (id, key, label),
        };
        self.wwise_deferred = None; // FMOD playback supersedes a pending event

        let Some(tags_root) = tags_root else {
            self.status = Some("no source loaded".to_owned());
            return;
        };

        let Some(banks) = self.ensure_banks(tags_root) else {
            self.status = Some("no FMOD bank under <game>/fmod/pc".to_owned());
            return;
        };
        let Some((bank, sub)) = resolve_bank(banks, id, &key) else {
            self.status = Some(format!("'{label}' not found in FMOD bank"));
            return;
        };
        if let Some(pcm) = self.cache.get(&(bank, sub)) {
            self.play_request += 1;
            self.play_decoded(&pcm, &label);
            return;
        }
        let banks = self.banks.clone().expect("opened above");
        let cache = PcmKey::Bank {
            generation: self.banks_generation,
            bank,
            sub,
        };
        self.spawn_decode(label, Some(cache), ctx, move || {
            decode_bank_subsound(&banks, bank, sub)
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(samples: usize) -> Arc<DecodedPcm> {
        Arc::new(DecodedPcm {
            samples: vec![0; samples],
            channels: 1,
            sample_rate: 48_000,
        })
    }

    /// Every audition used to stay decoded for the rest of the session.
    #[test]
    fn the_pcm_cache_drops_the_least_recently_played_first() {
        let mut cache = PcmCache::<u32> {
            budget: 250,
            ..Default::default()
        };
        cache.insert(1, pcm(50)); // 100 bytes each
        cache.insert(2, pcm(50));
        assert!(cache.get(&1).is_some(), "replaying 1 makes 2 the oldest");
        cache.insert(3, pcm(50));
        assert!(cache.get(&2).is_none(), "over budget: 2 went");
        assert!(cache.get(&1).is_some() && cache.get(&3).is_some());
        assert_eq!(cache.bytes, 200);
    }

    fn decoded(request: u64, cache: Option<PcmKey>) -> AudioDone {
        AudioDone::Decoded {
            request,
            cache,
            label: "rifle_fire".to_owned(),
            result: Ok(pcm(8)),
        }
    }

    /// A decode that lands after the user moved on is kept for next time but
    /// not played over what they are listening to now.
    #[test]
    fn a_superseded_decode_is_cached_but_not_played() {
        let mut audio = AudioState {
            // No output device, so playing says so instead of opening one.
            engine_tried: true,
            play_request: 2,
            ..Default::default()
        };
        audio.jobs.running = 1;
        let key = || PcmKey::Bank {
            generation: 0,
            bank: 0,
            sub: 4,
        };
        audio.apply_job(decoded(1, Some(key())));
        assert_eq!(audio.status, None, "request 1 is stale: not played");
        assert!(audio.cache.get(&(0, 4)).is_some(), "but cached");

        audio.apply_job(decoded(2, None));
        assert_eq!(audio.status.as_deref(), Some("no audio output device"));
        assert_eq!(audio.jobs.running, 0);
    }

    /// Bank indices mean nothing once the banks reopen (a language change).
    #[test]
    fn a_decode_for_reopened_banks_is_not_cached() {
        let mut audio = AudioState {
            engine_tried: true,
            banks_generation: 3,
            ..Default::default()
        };
        audio.apply_job(decoded(
            0,
            Some(PcmKey::Bank {
                generation: 2,
                bank: 0,
                sub: 4,
            }),
        ));
        assert!(audio.cache.get(&(0, 4)).is_none());
    }
}
