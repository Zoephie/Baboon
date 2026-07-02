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

/// An audition action queued by the sound-player UI, drained each frame by
/// [`AudioState::process`].
/// A codec for tag-inline audio (classic CE/H2). Ogg Vorbis is self-describing;
/// Opus/Xbox-ADPCM need the channel count (and ADPCM the sample rate) supplied
/// from the tag, since their raw streams don't carry it.
#[derive(Clone, Copy)]
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
    PlayInline {
        bytes: Vec<u8>,
        codec: InlineCodec,
        channels: u16,
        sample_rate: u32,
        label: String,
    },
    /// Play a Wwise event by name (Halo 4). The audio lives in
    /// `<game>/sound/pc/*.pck`; the tag only carries the event name.
    PlayEvent { event_name: String, label: String },
    /// Set the playback volume (linear amplitude, 0.0..=1.0). Applies to every
    /// live voice immediately and to all subsequent plays.
    SetVolume(f32),
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
    cache: HashMap<(usize, usize), Arc<DecodedPcm>>,
    /// Lazily-opened Wwise packages (Halo 4) + a decoded-event cache. The index
    /// is built on a background thread (`wwise_loading`), since it reads every
    /// bank; `wwise_root` marks which source it belongs to. `None` after a load
    /// that found no packages.
    wwise: Option<WwiseBanks>,
    wwise_root: Option<PathBuf>,
    /// In-flight background index build: the source root it's for, and the
    /// channel it will deliver the opened banks (or `None`) on.
    wwise_loading: Option<(PathBuf, Receiver<Option<WwiseBanks>>)>,
    /// An event queued to play as soon as the in-flight load finishes.
    wwise_deferred: Option<(String, String)>,
    event_cache: HashMap<String, Arc<DecodedPcm>>,
    /// Current playback volume (linear, 0.0..=1.0). Held here so it survives
    /// before the engine is lazily created and seeds it on first play.
    volume: Volume,
    /// Set by the sound-player UI; drained by [`AudioState::process`].
    pub(super) pending: Option<SoundAction>,
    /// Last user-facing status line (bank/resolve/playback result).
    pub(super) status: Option<String>,
}

impl AudioState {
    /// Lazily open every FMOD bank under `<game>/fmod/pc/` for this source.
    fn ensure_banks(&mut self, tags_root: &Path) -> Option<&SoundBanks> {
        if self.banks_root.as_deref() != Some(tags_root) {
            self.banks = SoundBanks::open_pc(tags_root).ok();
            self.banks_root = Some(tags_root.to_path_buf());
            self.cache.clear();
        }
        self.banks.as_ref()
    }

    /// Kick off a background build of the Wwise index for `tags_root` (unless
    /// one is already in flight for the same root). Reads every bank to build
    /// the event graph, so it must not run on the UI thread. `ctx` is pinged
    /// when it finishes so the drain loop picks up the result promptly.
    fn start_wwise_load(&mut self, tags_root: &Path, ctx: &egui::Context) {
        if self.wwise_loading.as_ref().map(|(r, _)| r.as_path()) == Some(tags_root) {
            return; // already loading this root
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let root = tags_root.to_path_buf();
        let thread_root = root.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let banks = WwiseBanks::open_pc(&thread_root).ok();
            let _ = tx.send(banks);
            ctx.request_repaint();
        });
        self.wwise_loading = Some((root, rx));
    }

    /// Poll the in-flight Wwise load; on completion, store the banks and play
    /// any event that was deferred while it built. Returns early (borrow
    /// released) while the load is still running.
    fn poll_wwise_load(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let banks = match self.wwise_loading.as_ref() {
            Some((_, rx)) => match rx.try_recv() {
                Ok(banks) => banks,                    // finished (Some/None banks)
                Err(TryRecvError::Empty) => return,    // still loading
                Err(TryRecvError::Disconnected) => None, // worker died
            },
            None => return,
        };
        let root = self.wwise_loading.take().map(|(r, _)| r);
        let ok = banks.is_some();
        self.wwise = banks;
        self.wwise_root = root;
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
                label,
            } => {
                self.wwise_deferred = None; // superseded by this playback
                // Classic CE/H2: audio is inline in the tag.
                let decoded = match codec {
                    InlineCodec::OggVorbis => blam_tags::audio::decode_ogg_vorbis(&bytes),
                    InlineCodec::Opus => blam_tags::audio::decode_opus(&bytes, channels),
                    InlineCodec::XboxAdpcm => {
                        blam_tags::audio::decode_xbox_adpcm(&bytes, channels, sample_rate)
                    }
                    InlineCodec::Pcm { big_endian } => {
                        blam_tags::audio::decode_pcm(&bytes, channels, sample_rate, big_endian)
                    }
                };
                match decoded {
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
                // Banks already built for this source? Resolve + play now.
                if self.wwise_root.as_deref() == Some(tags_root) {
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

        // Resolve the permutation name to a (bank, subsound). Borrow ends here.
        let resolved = match self.ensure_banks(tags_root) {
            Some(banks) => banks.resolve(&key),
            None => {
                self.status = Some("no FMOD bank under <game>/fmod/pc".to_owned());
                return;
            }
        };
        let Some((bank_index, sub_index)) = resolved else {
            self.status = Some(format!("'{label}' not found in FMOD bank"));
            return;
        };

        // Decode (cached). The bank borrow is scoped so it drops before the
        // cache insert / engine play below.
        let pcm = match self.cache.get(&(bank_index, sub_index)) {
            Some(pcm) => pcm.clone(),
            None => {
                let decoded = {
                    let banks = self.banks.as_ref().expect("banks opened above");
                    let bank = banks.bank(bank_index);
                    let sub = &bank.subsounds[sub_index];
                    match bank.read_subsound_data(sub_index) {
                        Ok(data) => {
                            decode_subsound(&data, sub.channels, sub.frequency, sub.setup_hash)
                        }
                        Err(err) => Err(err),
                    }
                };
                match decoded {
                    Ok(pcm) => {
                        let pcm = Arc::new(pcm);
                        self.cache.insert((bank_index, sub_index), pcm.clone());
                        pcm
                    }
                    Err(err) => {
                        self.status = Some(format!("decode failed: {err}"));
                        return;
                    }
                }
            }
        };

        self.play_decoded(&pcm, &label);
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
