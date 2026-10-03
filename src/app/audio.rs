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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use blam_tags::audio::{DecodedPcm, SoundBanks, WwiseBanks, decode_subsound, downmix_to_stereo};
use eframe::egui;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};

use super::kit::KitId;

mod waveform;
pub(super) use waveform::{Waveform, channel_labels};

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
        /// The editing-kit tags root that owns the tag which queued playback.
        /// This must travel with the action: the globally active kit can change
        /// while a pane from another kit remains open.
        tags_root: Option<PathBuf>,
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
    PlayEvent {
        event_name: String,
        label: String,
        tags_root: Option<PathBuf>,
    },
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
    /// Stop and rewind the sound.
    Stop,
    /// Pause the sound if it is playing, else play it from where it is (from
    /// the start once it has ended).
    TogglePause,
    /// Move the playhead to this many seconds in. Playing continues from
    /// there, which is what makes dragging the playhead scrub.
    Seek(f64),
    /// Loop the sound (and every sound played after) or not.
    SetLooping(bool),
    /// Play only `start..end` seconds of the requesting tab's clip, or all of
    /// it. Remembered for the tab and clip, so it holds for a voice that
    /// starts later.
    SetRegion(Option<(f64, f64)>),
}

/// The tag tab a sound was started from. Playback follows its tab: it pauses
/// when another tab takes focus and is disposed of when the tab closes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct SoundOwner {
    pub(super) kit: KitId,
    pub(super) key: String,
}

/// A queued sound-player action and the tab that queued it (`None` for a view
/// with no tab of its own, whose sound nothing pauses or disposes of).
pub(super) struct SoundRequest {
    pub(super) owner: Option<SoundOwner>,
    /// Which of the tab's clips a play is for (a permutation, an event), so
    /// the player knows whether the clip it has selected is the one loaded.
    pub(super) clip: Option<String>,
    /// Decode the clip for its waveform without playing it.
    pub(super) preview: bool,
    pub(super) action: SoundAction,
}

/// A tab's selected clip decoded ahead of playing, so its player can show
/// the waveform before anything plays. Kept apart from the voice: previewing
/// never unloads another tab's sound.
#[derive(Clone)]
pub(in crate::app) struct Preview {
    pub(in crate::app) clip: String,
    pub(in crate::app) state: PreviewState,
}

#[derive(Clone)]
pub(in crate::app) enum PreviewState {
    Pending,
    Ready(Arc<Waveform>),
    /// Why there is no waveform: the reason a play would have reported.
    Failed(String),
}

impl From<SoundAction> for SoundRequest {
    /// An action from no tab.
    fn from(action: SoundAction) -> Self {
        Self {
            owner: None,
            clip: None,
            preview: false,
            action,
        }
    }
}

/// Where a pane queues its sound-player actions: each one is stamped with the
/// pane's tab, so the player never has to pass it along.
pub(in crate::app) struct SoundRequests<'a> {
    queue: &'a mut VecDeque<SoundRequest>,
    owner: Option<SoundOwner>,
}

impl<'a> SoundRequests<'a> {
    pub(in crate::app) fn new(
        queue: &'a mut VecDeque<SoundRequest>,
        owner: Option<SoundOwner>,
    ) -> Self {
        Self { queue, owner }
    }

    pub(in crate::app) fn push_back(&mut self, action: SoundAction) {
        self.queue.push_back(SoundRequest {
            owner: self.owner.clone(),
            clip: None,
            preview: false,
            action,
        });
    }

    /// Whether a play for `clip` from this tab is already queued.
    pub(in crate::app) fn queued_clip(&self, clip: &str) -> bool {
        self.queue.iter().any(|request| {
            !request.preview && request.owner == self.owner && request.clip.as_deref() == Some(clip)
        })
    }

    /// Queue a play for one of the tab's clips.
    pub(in crate::app) fn play_clip(&mut self, action: SoundAction, clip: String) {
        self.queue.push_back(SoundRequest {
            owner: self.owner.clone(),
            clip: Some(clip),
            preview: false,
            action,
        });
    }

    /// Queue an action about one of the tab's clips (its region) without
    /// playing it.
    pub(in crate::app) fn push_for_clip(&mut self, action: SoundAction, clip: String) {
        self.queue.push_back(SoundRequest {
            owner: self.owner.clone(),
            clip: Some(clip),
            preview: false,
            action,
        });
    }

    /// Queue a decode of one of the tab's clips for its waveform alone.
    pub(in crate::app) fn preview_clip(&mut self, action: SoundAction, clip: String) {
        self.queue.push_back(SoundRequest {
            owner: self.owner.clone(),
            clip: Some(clip),
            preview: true,
            action,
        });
    }
}

/// What the player shows of the sound its tab owns.
#[derive(Clone)]
pub(in crate::app) struct PlaybackView {
    pub(in crate::app) label: String,
    /// The clip it was played as, if the player named one.
    pub(in crate::app) clip: Option<String>,
    pub(in crate::app) position: f64,
    pub(in crate::app) duration: f64,
    pub(in crate::app) playing: bool,
    pub(in crate::app) looping: bool,
    /// The sound as decoded — every channel, before any fold to stereo — and
    /// its summaries, for the waveform.
    pub(in crate::app) waveform: Arc<Waveform>,
}

impl std::fmt::Debug for PlaybackView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackView")
            .field("label", &self.label)
            .field("clip", &self.clip)
            .field("position", &self.position)
            .field("duration", &self.duration)
            .field("playing", &self.playing)
            .field("looping", &self.looping)
            .finish_non_exhaustive()
    }
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

/// The label a play action shows.
fn action_label(action: &SoundAction) -> Option<String> {
    match action {
        SoundAction::Play { label, .. }
        | SoundAction::PlayInline { label, .. }
        | SoundAction::PlayEvent { label, .. }
        | SoundAction::PlayCeMedia { label, .. } => Some(label.clone()),
        _ => None,
    }
}

/// Where Play starts: the playhead while it is inside `region` (frames,
/// `start..end`), else the region's start.
fn play_from(position: u64, (start, end): (u64, u64)) -> u64 {
    if (start..end).contains(&position) {
        position
    } else {
        start
    }
}

/// The rodio output device.
struct Engine {
    handle: OutputStreamHandle,
    _stream: OutputStream,
}

impl Engine {
    fn new() -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Some(Self {
                handle,
                _stream: stream,
            }),
            Err(_) => None,
        }
    }
}

/// The play position a voice's source and the UI share. The source is the
/// only writer of `frame` while it plays; a seek is handed to it through
/// `seek` (and shown at once through `frame`).
struct PlaybackShared {
    /// The next frame the source will play.
    frame: AtomicU64,
    /// A frame to jump to, or `NO_SEEK`.
    seek: AtomicU64,
    looping: AtomicBool,
    /// The region played, `region_start..region_end` in frames; the whole
    /// sound while `region_end` is `NO_REGION`.
    region_start: AtomicU64,
    region_end: AtomicU64,
}

const NO_REGION: u64 = u64::MAX;

impl PlaybackShared {
    fn new(looping: bool) -> Self {
        Self {
            frame: AtomicU64::new(0),
            seek: AtomicU64::new(NO_SEEK),
            looping: AtomicBool::new(looping),
            region_start: AtomicU64::new(0),
            region_end: AtomicU64::new(NO_REGION),
        }
    }

    /// The frames played, `start..end`, within a sound of `frames` frames.
    fn region(&self, frames: u64) -> (u64, u64) {
        let end = self.region_end.load(Ordering::Relaxed);
        if end == NO_REGION {
            return (0, frames);
        }
        let end = end.min(frames);
        (self.region_start.load(Ordering::Relaxed).min(end), end)
    }
}

const NO_SEEK: u64 = u64::MAX;

/// A decoded sound played straight out of the shared buffer — no copy per
/// play — reading and advancing [`PlaybackShared`] so the UI sees where it is
/// and can move it.
struct PcmSource {
    pcm: Arc<DecodedPcm>,
    shared: Arc<PlaybackShared>,
    channels: u16,
    frames: u64,
    frame: u64,
    channel: u16,
}

impl Iterator for PcmSource {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        if self.channel == 0 {
            let seek = self.shared.seek.swap(NO_SEEK, Ordering::Relaxed);
            if seek != NO_SEEK {
                self.frame = seek.min(self.frames);
            }
            // The end of the region (or of the sound): wrap to its start when
            // looping, else stop there.
            let (start, end) = self.shared.region(self.frames);
            if self.frame >= end {
                if !self.shared.looping.load(Ordering::Relaxed) || end <= start {
                    self.shared.frame.store(end, Ordering::Relaxed);
                    return None;
                }
                self.frame = start;
            }
        }
        let sample =
            self.pcm.samples[(self.frame * self.channels as u64 + self.channel as u64) as usize];
        self.channel += 1;
        if self.channel == self.channels {
            self.channel = 0;
            self.frame += 1;
            self.shared.frame.store(self.frame, Ordering::Relaxed);
        }
        Some(sample)
    }
}

impl Source for PcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.pcm.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// The one sound loaded for playing: what it is, whose it is, and — while it
/// plays or sits paused — its output sink. It outlives its sink, so a sound
/// that has finished can be played again or sought from where it stands.
struct Voice {
    owner: Option<SoundOwner>,
    clip: Option<String>,
    label: String,
    /// The sound as decoded, with its summaries.
    waveform: Arc<Waveform>,
    /// The audio as played: at most stereo, so the output device takes it.
    pcm: Arc<DecodedPcm>,
    shared: Arc<PlaybackShared>,
    sink: Option<Sink>,
}

impl Voice {
    fn new(
        waveform: Arc<Waveform>,
        label: String,
        owner: Option<SoundOwner>,
        clip: Option<String>,
        looping: bool,
    ) -> Self {
        // Fold >2 channels down to stereo for the output device.
        let decoded = waveform.pcm();
        let pcm = if decoded.channels > 2 {
            Arc::new(DecodedPcm {
                samples: downmix_to_stereo(&decoded.samples, decoded.channels as usize),
                channels: 2,
                sample_rate: decoded.sample_rate,
            })
        } else {
            decoded.clone()
        };
        Self {
            owner,
            clip,
            label,
            waveform,
            pcm,
            shared: Arc::new(PlaybackShared::new(looping)),
            sink: None,
        }
    }

    fn frames(&self) -> u64 {
        self.pcm.frame_count() as u64
    }

    fn position(&self) -> u64 {
        self.shared.frame.load(Ordering::Relaxed).min(self.frames())
    }

    fn is_playing(&self) -> bool {
        self.sink
            .as_ref()
            .is_some_and(|sink| !sink.empty() && !sink.is_paused())
    }

    /// Play from where the playhead is — from the start once it has reached
    /// the end — on a new sink if the last one has run dry.
    fn play(&mut self, engine: &Engine, volume: f32) {
        // From the playhead while it is inside the region (the whole sound
        // when there is none), else from the region's start.
        let position = self.position();
        let start = play_from(position, self.shared.region(self.frames()));
        if let Some(sink) = self.sink.as_ref().filter(|sink| !sink.empty()) {
            if start != position {
                self.seek(start);
            }
            sink.play();
            return;
        }
        if self.pcm.channels == 0 || self.frames() == 0 {
            return;
        }
        self.shared.seek.store(NO_SEEK, Ordering::Relaxed);
        self.shared.frame.store(start, Ordering::Relaxed);
        let Ok(sink) = Sink::try_new(&engine.handle) else {
            return;
        };
        sink.set_volume(volume);
        sink.append(PcmSource {
            pcm: self.pcm.clone(),
            shared: self.shared.clone(),
            channels: self.pcm.channels,
            frames: self.frames(),
            frame: start,
            channel: 0,
        });
        self.sink = Some(sink);
    }

    fn pause(&self) {
        if let Some(sink) = &self.sink {
            sink.pause();
        }
    }

    /// Move the playhead. A playing sound carries on from there; a paused or
    /// finished one waits there.
    fn seek(&self, frame: u64) {
        let frame = frame.min(self.frames());
        self.shared.frame.store(frame, Ordering::Relaxed);
        if self.sink.as_ref().is_some_and(|sink| !sink.empty()) {
            self.shared.seek.store(frame, Ordering::Relaxed);
        }
    }

    /// Stop and rewind.
    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.shared.seek.store(NO_SEEK, Ordering::Relaxed);
        self.shared.frame.store(0, Ordering::Relaxed);
    }

    /// Play only `start..end` seconds (all of it for `None`).
    fn set_region(&self, region: Option<(f64, f64)>) {
        let rate = f64::from(self.pcm.sample_rate.max(1));
        match region {
            Some((start, end)) => {
                let (start, end) = (start.min(end).max(0.0), start.max(end).max(0.0));
                self.shared
                    .region_start
                    .store((start * rate) as u64, Ordering::Relaxed);
                self.shared
                    .region_end
                    .store((end * rate).ceil() as u64, Ordering::Relaxed);
            }
            None => self.shared.region_end.store(NO_REGION, Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    fn region(&self) -> Option<(f64, f64)> {
        if self.shared.region_end.load(Ordering::Relaxed) == NO_REGION {
            return None;
        }
        let rate = f64::from(self.pcm.sample_rate.max(1));
        let (start, end) = self.shared.region(self.frames());
        Some((start as f64 / rate, end as f64 / rate))
    }

    fn view(&self) -> PlaybackView {
        let rate = self.pcm.sample_rate.max(1) as f64;
        PlaybackView {
            label: self.label.clone(),
            clip: self.clip.clone(),
            position: self.position() as f64 / rate,
            duration: self.frames() as f64 / rate,
            playing: self.is_playing(),
            looping: self.shared.looping.load(Ordering::Relaxed),
            waveform: self.waveform.clone(),
        }
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.stop();
    }
}

/// App-owned audio state. Everything is lazy: the output device opens on the
/// first play, the banks open on the first resolve for a given source.
#[derive(Default)]
pub(super) struct AudioState {
    /// The sound loaded for playing. Declared before `engine` so its sink is
    /// dropped before the output stream.
    voice: Option<Voice>,
    /// Whether sounds loop; carried from one voice to the next.
    looping: bool,
    /// The tab the action being processed came from, and the clip it names.
    request_owner: Option<SoundOwner>,
    request_clip: Option<String>,
    /// Whether the action being processed is a preview.
    request_preview: bool,
    /// Whether processing a preview started a decode for it.
    preview_spawned: bool,
    /// Each tab's preview of its selected clip.
    previews: HashMap<SoundOwner, Preview>,
    /// Each tab's region, in seconds, and the clip it is on.
    regions: HashMap<SoundOwner, (String, (f64, f64))>,
    /// The tab the newest decode was started for, so closing it cancels the
    /// decode as well as the sound.
    decode_owner: Option<SoundOwner>,
    engine: Option<Engine>,
    engine_tried: bool,
    banks: Option<Arc<SoundBanks>>,
    /// Why the current FMOD bank set could not be opened. This is retained
    /// with the negative cache so the player can report the actual path or
    /// format problem on every attempt.
    banks_error: Option<String>,
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
    /// An event queued to play as soon as the in-flight load finishes, and
    /// the tab it is for.
    wwise_deferred: Option<(String, String, Option<SoundOwner>, Option<String>)>,
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
    pub(super) pending: VecDeque<SoundRequest>,
    /// Last user-facing status line (bank/resolve/playback result).
    pub(super) status: Option<String>,
    /// The tab whose action produced `status`; `None` for one no tab did.
    /// Only that tab's player shows it, so a failure in one kit does not
    /// read as another's.
    status_owner: Option<SoundOwner>,
    /// Decodes and extraction batches running on workers report back here.
    jobs: AudioJobs,
    /// The playback most recently asked for. A decode finishing for an older
    /// one is cached but not played: the user has moved on, or pressed Stop.
    play_request: u64,
}

/// Decoded audio kept for replay, bounded by size. Once over the budget the
/// least recently played entries go first.
struct PcmCache<K> {
    map: HashMap<K, Arc<Waveform>>,
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

impl<K: Hash + Eq + Clone> PcmCache<K> {
    fn get(&mut self, key: &K) -> Option<Arc<Waveform>> {
        let pcm = self.map.get(key)?.clone();
        if let Some(position) = self.order.iter().position(|entry| entry == key)
            && let Some(entry) = self.order.remove(position)
        {
            self.order.push_back(entry);
        }
        Some(pcm)
    }

    fn insert(&mut self, key: K, waveform: Arc<Waveform>) {
        if let Some(old) = self.map.insert(key.clone(), waveform.clone()) {
            self.bytes -= old.bytes();
            self.order.retain(|entry| entry != &key);
        }
        self.bytes += waveform.bytes();
        self.order.push_back(key);
        // The newest entry stays even when it alone is over budget: it is
        // the one about to be replayed.
        while self.bytes > self.budget && self.order.len() > 1 {
            if let Some(evicted) = self.order.pop_front()
                && let Some(old) = self.map.remove(&evicted)
            {
                self.bytes -= old.bytes();
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
    Bank {
        generation: u64,
        bank: usize,
        sub: usize,
    },
    Event {
        generation: u64,
        name: String,
    },
}

/// A worker's report.
enum AudioDone {
    Decoded {
        request: u64,
        cache: Option<PcmKey>,
        label: String,
        owner: Option<SoundOwner>,
        clip: Option<String>,
        preview: bool,
        result: Result<Arc<Waveform>, String>,
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
    tags_root: Option<PathBuf>,
    ce_media: Arc<Mutex<crate::source::ce_audio::CeMediaStore>>,
}

/// Open every FMOD bank set a request needs and prove that each requested
/// permutation resolves before extraction creates any output. This is run once
/// at the UI boundary for immediate feedback and again on the worker to guard
/// against the banks being moved between the click and the write.
fn preflight_fmod_banks(
    request: &ExtractRequest,
    tags_root: Option<&Path>,
) -> Result<HashMap<Option<String>, SoundBanks>, String> {
    let mut banks_by_language = HashMap::new();
    for item in &request.items {
        let ExtractSource::Bank { language, .. } = &item.source else {
            continue;
        };
        if banks_by_language.contains_key(language) {
            continue;
        }
        let Some(tags_root) = tags_root else {
            return Err("no editing-kit tags root is available for the FMOD banks".to_owned());
        };
        let banks =
            SoundBanks::open_pc_language(tags_root, language.as_deref()).map_err(|error| {
                let qualifier = language
                    .as_deref()
                    .map(|language| format!(" for {language}"))
                    .unwrap_or_default();
                format!("FMOD banks{qualifier} are unavailable: {error}")
            })?;
        banks_by_language.insert(language.clone(), banks);
    }
    for item in &request.items {
        let ExtractSource::Bank { id, key, language } = &item.source else {
            continue;
        };
        let Some(banks) = banks_by_language.get(language) else {
            return Err("the required FMOD bank was not opened".to_owned());
        };
        if resolve_bank(banks, *id, key).is_none() {
            let language = language.as_deref().unwrap_or("default");
            return Err(format!(
                "'{key}' was not found in the {language} FMOD banks (output: {})",
                item.out_path.display()
            ));
        }
    }
    Ok(banks_by_language)
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
    // A bulk request may span several localized bank sets. Validate every FMOD
    // dependency before writing the first output, so a missing/wrong H3-family
    // bank cancels atomically instead of leaving a partial export behind.
    let fmod_by_language = match preflight_fmod_banks(&request, sources.tags_root.as_deref()) {
        Ok(banks) => banks,
        Err(error) => return format!("Extraction cancelled — {error}"),
    };

    let mut wwise_by_language: HashMap<Option<String>, Option<WwiseBanks>> = HashMap::new();
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
            ExtractSource::Bank { id, key, language } => {
                let banks = fmod_by_language
                    .get(&language)
                    .expect("FMOD extraction dependencies were preflighted");
                match resolve_bank(banks, id, &key) {
                    None => Err(format!("'{key}' not in bank")),
                    Some((bank, sub)) => decode_bank_subsound(banks, bank, sub)
                        .and_then(|pcm| write(&item.out_path, &pcm)),
                }
            }
            ExtractSource::CeMedia { paks_root, media } => {
                decode_ce_media(&sources.ce_media, &paks_root, &media)
                    .and_then(|pcm| write(&item.out_path, &pcm))
            }
            ExtractSource::Event { name, language } => {
                let banks = wwise_by_language
                    .entry(language.clone())
                    .or_insert_with(|| {
                        sources.tags_root.as_deref().and_then(|root| {
                            WwiseBanks::open_pc_language(root, language.as_deref()).ok()
                        })
                    })
                    .as_ref();
                match banks {
                    Some(banks) => banks
                        .resolve(&name)
                        .and_then(|pcm| write(&item.out_path, &pcm)),
                    None => Err(format!(
                        "no Wwise bank{}",
                        language
                            .as_deref()
                            .map(|language| format!(" for {language}"))
                            .unwrap_or_default()
                    )),
                }
            }
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
    fn ensure_banks(&mut self, tags_root: &Path) -> Result<&SoundBanks, String> {
        let lang = self.language.clone();
        if self.banks.is_none()
            || self.banks_root.as_deref() != Some(tags_root)
            || self.banks_lang.as_ref() != Some(&lang)
        {
            match SoundBanks::open_pc_language(tags_root, lang.as_deref()) {
                Ok(banks) => {
                    self.banks = Some(Arc::new(banks));
                    self.banks_error = None;
                }
                Err(error) => {
                    self.banks = None;
                    self.banks_error = Some(error);
                }
            }
            self.banks_root = Some(tags_root.to_path_buf());
            self.banks_lang = Some(lang);
            self.banks_generation += 1;
            self.cache.clear();
        }
        self.banks.as_deref().ok_or_else(|| {
            self.banks_error
                .clone()
                .unwrap_or_else(|| "the FMOD banks could not be opened".to_owned())
        })
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
        let owner = self.request_owner.clone();
        let clip = self.request_clip.clone();
        let preview = self.request_preview;
        if preview {
            // Not a play: it must not supersede one or say anything.
            self.preview_spawned = true;
        } else {
            self.play_request += 1;
            self.decode_owner = owner.clone();
            self.status = Some(format!("decoding {label}\u{2026}"));
        }
        let request = self.play_request;
        self.spawn_job(ctx, move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode))
                .unwrap_or_else(|panic| Err(super::state::panic_text(&panic)))
                // Summarised here, off the UI thread, and cached with the audio.
                .map(|pcm| Arc::new(Waveform::new(Arc::new(pcm))));
            AudioDone::Decoded {
                request,
                cache,
                label,
                owner,
                clip,
                preview,
                result,
            }
        });
    }

    fn spawn_job(&mut self, ctx: &egui::Context, job: impl FnOnce() -> AudioDone + Send + 'static) {
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
            AudioDone::Extracted(status) => {
                self.status = Some(status);
                self.status_owner = None;
            }
            AudioDone::Decoded {
                request,
                cache,
                label,
                owner,
                clip,
                preview,
                result,
            } => {
                if let Ok(pcm) = &result {
                    self.cache_decoded(cache, pcm);
                }
                if preview {
                    if let (Some(owner), Some(clip)) = (owner, clip)
                        && let Some(slot) = self.previews.get_mut(&owner)
                        && slot.clip == clip
                        && matches!(slot.state, PreviewState::Pending)
                    {
                        slot.state = match result {
                            Ok(waveform) => PreviewState::Ready(waveform),
                            Err(error) => PreviewState::Failed(format!("decode failed: {error}")),
                        };
                    }
                    return;
                }
                let pcm = match result {
                    Ok(pcm) => pcm,
                    Err(error) => {
                        if request == self.play_request {
                            self.status = Some(format!("decode failed: {error}"));
                            self.status_owner = owner;
                        }
                        return;
                    }
                };
                if request == self.play_request {
                    self.play_decoded(pcm, &label, owner, clip);
                }
            }
        }
    }

    /// Keep a finished decode for replay, if the banks it came from are still
    /// the open ones.
    fn cache_decoded(&mut self, cache: Option<PcmKey>, waveform: &Arc<Waveform>) {
        match cache {
            Some(PcmKey::Bank {
                generation,
                bank,
                sub,
            }) if generation == self.banks_generation => {
                self.cache.insert((bank, sub), waveform.clone());
            }
            Some(PcmKey::Event { generation, name }) if generation == self.wwise_generation => {
                self.event_cache.insert(name, waveform.clone());
            }
            _ => {}
        }
    }

    /// Hand over an already-decoded sound: as the tab's preview when that is
    /// what was asked for, else played.
    fn deliver(&mut self, waveform: Arc<Waveform>, label: &str) {
        if self.request_preview {
            if let (Some(owner), Some(clip)) =
                (self.request_owner.clone(), self.request_clip.clone())
            {
                self.previews.insert(
                    owner,
                    Preview {
                        clip,
                        state: PreviewState::Ready(waveform),
                    },
                );
                self.preview_spawned = true;
            }
            return;
        }
        self.play_request += 1;
        self.play_decoded(
            waveform,
            label,
            self.request_owner.clone(),
            self.request_clip.clone(),
        );
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
        if let Err(error) = preflight_fmod_banks(&request, request.tags_root.as_deref()) {
            self.status = Some(format!("Extraction cancelled — {error}"));
            return;
        }
        let sources = ExtractSources {
            tags_root: request.tags_root.clone(),
            ce_media: self.ce_media.clone(),
        };
        self.status = Some(format!("extracting {}\u{2026}", request.label));
        self.status_owner = None;
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
            Some((event_name, label, owner, clip)) => {
                self.request_owner = owner;
                self.request_clip = clip;
                self.request_preview = false;
                self.play_event(&event_name, &label, ctx);
            }
            None if !ok => self.status = Some("no Wwise .pck under <game>/sound/pc".to_owned()),
            None => {}
        }
    }

    /// Resolve an event name to PCM (cached) and play it. Assumes the banks for
    /// the current source are already loaded (`wwise_root` set).
    fn play_event(&mut self, event_name: &str, label: &str, ctx: &egui::Context) {
        if let Some(pcm) = self.event_cache.get(&event_name.to_owned()) {
            self.deliver(pcm, label);
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

    /// The current playback volume (linear, 0.0..=1.0), for the UI slider.
    pub(super) fn volume(&self) -> f32 {
        self.volume.0
    }

    fn ensure_engine(&mut self) -> Option<&Engine> {
        if !self.engine_tried {
            self.engine = Engine::new();
            self.engine_tried = true;
        }
        self.engine.as_ref()
    }

    /// The sound `owner`'s tab has loaded, for its player.
    pub(super) fn playback(&self, owner: Option<&SoundOwner>) -> Option<PlaybackView> {
        self.voice
            .as_ref()
            .filter(|voice| voice.owner.as_ref() == owner)
            .map(Voice::view)
    }

    /// Whether sounds loop.
    pub(super) fn looping(&self) -> bool {
        self.looping
    }

    /// Whether a sound is playing — the UI repaints every frame while one is,
    /// to move its playhead.
    pub(super) fn is_playing(&self) -> bool {
        self.voice.as_ref().is_some_and(Voice::is_playing)
    }

    /// Keep playback with its tab: pause it once `focus` is another tab (or
    /// none), and dispose of it — with any decode or Wwise load still on its
    /// way for it — once `is_open` says its tab is gone. A sound with no tab is
    /// left alone.
    pub(super) fn follow_tabs(
        &mut self,
        focus: Option<&SoundOwner>,
        is_open: impl Fn(&SoundOwner) -> bool,
    ) {
        let closed =
            |owner: &Option<SoundOwner>| owner.as_ref().is_some_and(|owner| !is_open(owner));
        if closed(&self.decode_owner) {
            self.play_request += 1;
            self.decode_owner = None;
        }
        self.previews.retain(|owner, _| is_open(owner));
        self.regions.retain(|owner, _| is_open(owner));
        if closed(&self.status_owner) {
            self.status = None;
            self.status_owner = None;
        }
        if self
            .wwise_deferred
            .as_ref()
            .is_some_and(|(_, _, owner, _)| closed(owner))
        {
            self.wwise_deferred = None;
        }
        if self
            .voice
            .as_ref()
            .is_some_and(|voice| closed(&voice.owner))
        {
            self.voice = None;
            return;
        }
        if let Some(voice) = self.voice.as_ref()
            && let Some(owner) = voice.owner.as_ref()
            && Some(owner) != focus
            && voice.is_playing()
        {
            voice.pause();
        }
    }

    /// Whether the voice belongs to the tab this action came from: transport
    /// controls in one tab do not reach another tab's sound.
    fn voice_is_requesters(&self) -> bool {
        self.voice
            .as_ref()
            .is_some_and(|voice| voice.owner == self.request_owner)
    }

    /// Drain the pending UI action: resolve the subsound, decode (cached), play.
    pub(super) fn process(&mut self, tags_root: Option<&Path>, ctx: &egui::Context) {
        self.drain_jobs();
        // Pick up a finished background Wwise load (and play any deferred event).
        let before = self.status.clone();
        self.poll_wwise_load(ctx);
        self.claim_status(before);
        if self.is_playing() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        let Some(SoundRequest {
            owner,
            clip,
            preview,
            action,
        }) = self.pending.pop_front()
        else {
            return;
        };
        self.request_owner = owner;
        self.request_clip = clip;
        self.request_preview = preview;
        if preview {
            self.preview(action, tags_root, ctx);
            return;
        }
        let before = self.status.clone();
        // A clip its tab has already previewed plays as it is, undecoded.
        if let Some(waveform) = self.ready_preview()
            && let Some(label) = action_label(&action)
        {
            self.deliver(waveform, &label);
        } else {
            self.handle(action, tags_root, ctx);
        }
        self.claim_status(before);
    }

    /// The waveform the requesting tab has previewed for the requested clip.
    fn ready_preview(&self) -> Option<Arc<Waveform>> {
        let preview = self.previews.get(self.request_owner.as_ref()?)?;
        match &preview.state {
            PreviewState::Ready(waveform)
                if Some(preview.clip.as_str()) == self.request_clip.as_deref() =>
            {
                Some(waveform.clone())
            }
            _ => None,
        }
    }

    /// Resolve and decode a clip for the requesting tab's preview, the way a
    /// play would but with none of its effects: the status line, a pending
    /// play and a deferred Wwise play are all left as they were. A Wwise
    /// event whose banks are still loading is dropped, to be asked for again;
    /// anything else that went nowhere records why.
    fn preview(&mut self, action: SoundAction, tags_root: Option<&Path>, ctx: &egui::Context) {
        let (Some(owner), Some(clip)) = (self.request_owner.clone(), self.request_clip.clone())
        else {
            return;
        };
        self.previews.insert(
            owner.clone(),
            Preview {
                clip,
                state: PreviewState::Pending,
            },
        );
        let status = self.status.clone();
        let deferred = self.wwise_deferred.clone();
        let play_request = self.play_request;
        self.preview_spawned = false;
        self.handle(action, tags_root, ctx);
        let reason = (self.status != status)
            .then(|| self.status.clone())
            .flatten();
        self.status = status;
        self.wwise_deferred = deferred;
        self.play_request = play_request;
        self.request_preview = false;
        if !self.preview_spawned {
            if self.wwise_loading.is_some() {
                self.previews.remove(&owner);
            } else if let Some(slot) = self.previews.get_mut(&owner)
                && matches!(slot.state, PreviewState::Pending)
            {
                slot.state =
                    PreviewState::Failed(reason.unwrap_or_else(|| "no audio to show".to_owned()));
            }
        }
    }

    /// The requesting tab's preview, for its player.
    pub(super) fn preview_for(&self, owner: &SoundOwner) -> Option<&Preview> {
        self.previews.get(owner)
    }

    /// A status line the work just done changed belongs to the tab it was
    /// done for, whose player alone shows it.
    fn claim_status(&mut self, before: Option<String>) {
        if self.status != before {
            self.status_owner = self.request_owner.clone();
        }
    }

    /// Whether `owner`'s player shows the status line: one it caused, or one
    /// no tab did (an extraction's).
    pub(super) fn status_is_for(&self, owner: &SoundOwner) -> bool {
        self.status_owner
            .as_ref()
            .is_none_or(|status_owner| status_owner == owner)
    }

    fn handle(&mut self, action: SoundAction, tags_root: Option<&Path>, ctx: &egui::Context) {
        let (id, key, label, action_root) = match action {
            SoundAction::SetVolume(v) => {
                let v = v.clamp(0.0, 1.0);
                self.volume = Volume(v);
                if let Some(sink) = self.voice.as_ref().and_then(|voice| voice.sink.as_ref()) {
                    sink.set_volume(v);
                }
                return;
            }
            SoundAction::TogglePause => {
                if !self.voice_is_requesters() {
                    return;
                }
                if self.voice.as_ref().is_some_and(Voice::is_playing) {
                    self.voice.as_ref().expect("checked").pause();
                } else {
                    let volume = self.volume.0;
                    if self.ensure_engine().is_none() {
                        self.status = Some("no audio output device".to_owned());
                        return;
                    }
                    let engine = self.engine.as_ref().expect("ensured");
                    self.voice.as_mut().expect("checked").play(engine, volume);
                }
                ctx.request_repaint();
                return;
            }
            SoundAction::Seek(seconds) => {
                if self.voice_is_requesters()
                    && let Some(voice) = self.voice.as_ref()
                {
                    let frame = (seconds.max(0.0) * voice.pcm.sample_rate as f64) as u64;
                    voice.seek(frame);
                    ctx.request_repaint();
                }
                return;
            }
            SoundAction::SetRegion(region) => {
                let (Some(owner), Some(clip)) =
                    (self.request_owner.clone(), self.request_clip.clone())
                else {
                    return;
                };
                match region {
                    Some(region) => {
                        self.regions.insert(owner, (clip.clone(), region));
                    }
                    None => {
                        self.regions.remove(&owner);
                    }
                }
                if self.voice_is_requesters()
                    && let Some(voice) = self.voice.as_ref()
                    && voice.clip.as_deref() == Some(clip.as_str())
                {
                    voice.set_region(region);
                }
                ctx.request_repaint();
                return;
            }
            SoundAction::SetLooping(looping) => {
                self.looping = looping;
                if let Some(voice) = self.voice.as_ref() {
                    voice.shared.looping.store(looping, Ordering::Relaxed);
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
                if let Some(voice) = self.voice.as_mut() {
                    voice.stop();
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
            SoundAction::PlayEvent {
                event_name,
                label,
                tags_root: action_root,
            } => {
                let tags_root = action_root.as_deref().or(tags_root);
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
                    self.wwise_deferred = Some((
                        event_name,
                        label,
                        self.request_owner.clone(),
                        self.request_clip.clone(),
                    ));
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
            SoundAction::Play {
                id,
                key,
                label,
                tags_root,
            } => (id, key, label, tags_root),
        };
        self.wwise_deferred = None; // FMOD playback supersedes a pending event

        let tags_root = action_root.as_deref().or(tags_root);
        let Some(tags_root) = tags_root else {
            self.status = Some("no source loaded".to_owned());
            return;
        };

        let banks = match self.ensure_banks(tags_root) {
            Ok(banks) => banks,
            Err(error) => {
                self.status = Some(format!(
                    "FMOD audio unavailable: {error}. Add the appropriate .fsb banks under the editing kit's fmod\\pc folder."
                ));
                return;
            }
        };
        let Some((bank, sub)) = resolve_bank(banks, id, &key) else {
            let language = self.language.as_deref().unwrap_or("the default language");
            self.status = Some(format!(
                "FMOD audio unavailable: '{label}' was not found in the opened banks for {language}. The matching language bank or .fsb.info may be missing from the editing kit's fmod\\pc folder."
            ));
            return;
        };
        if let Some(pcm) = self.cache.get(&(bank, sub)) {
            self.deliver(pcm, &label);
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

    /// Load an already-decoded sound as the one voice — disposing of the
    /// last, whichever tab it was — and play it.
    fn play_decoded(
        &mut self,
        waveform: Arc<Waveform>,
        label: &str,
        owner: Option<SoundOwner>,
        clip: Option<String>,
    ) {
        let secs = waveform.duration_secs();
        self.voice = None;
        self.decode_owner = None;
        let volume = self.volume.0;
        if self.ensure_engine().is_none() {
            self.status = Some("no audio output device".to_owned());
            return;
        }
        let engine = self.engine.as_ref().expect("ensured");
        let mut voice = Voice::new(
            waveform,
            label.to_owned(),
            owner.clone(),
            clip,
            self.looping,
        );
        // A region the tab set on this clip before it was loaded holds.
        let region = owner
            .as_ref()
            .and_then(|owner| self.regions.get(owner))
            .filter(|(region_clip, _)| voice.clip.as_deref() == Some(region_clip.as_str()))
            .map(|(_, region)| *region);
        voice.set_region(region);
        voice.play(engine, volume);
        self.voice = Some(voice);
        self.status = Some(format!("\u{25B6} {label}  ({secs:.2}s)"));
        self.status_owner = owner;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued<const N: usize>(actions: [SoundAction; N]) -> VecDeque<SoundRequest> {
        actions.into_iter().map(SoundRequest::from).collect()
    }

    fn pcm(samples: usize) -> Arc<Waveform> {
        Arc::new(Waveform::new(Arc::new(DecodedPcm {
            samples: vec![0; samples],
            channels: 1,
            sample_rate: 48_000,
        })))
    }

    /// Every audition used to stay decoded for the rest of the session.
    #[test]
    fn the_pcm_cache_drops_the_least_recently_played_first() {
        let mut cache = PcmCache::<u32> {
            budget: 250,
            ..Default::default()
        };
        let each = pcm(50).bytes(); // 100 bytes of samples and a summary
        cache.budget = each * 2 + each / 2;
        cache.insert(1, pcm(50));
        cache.insert(2, pcm(50));
        assert!(cache.get(&1).is_some(), "replaying 1 makes 2 the oldest");
        cache.insert(3, pcm(50));
        assert!(cache.get(&2).is_none(), "over budget: 2 went");
        assert!(cache.get(&1).is_some() && cache.get(&3).is_some());
        assert_eq!(cache.bytes, each * 2);
    }

    fn decoded(request: u64, cache: Option<PcmKey>) -> AudioDone {
        AudioDone::Decoded {
            request,
            cache,
            label: "rifle_fire".to_owned(),
            owner: None,
            clip: None,
            preview: false,
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
        let key = || PcmKey::Event {
            generation: 0,
            name: "rifle_fire".to_owned(),
        };
        audio.apply_job(decoded(1, Some(key())));
        assert_eq!(audio.status, None, "request 1 is stale: not played");
        assert!(
            audio.event_cache.get(&"rifle_fire".to_owned()).is_some(),
            "but cached"
        );

        audio.apply_job(decoded(2, None));
        assert_eq!(audio.status.as_deref(), Some("no audio output device"));
        assert_eq!(audio.jobs.running, 0);
    }

    /// Event names must not be cached against a replacement Wwise bank set.
    #[test]
    fn a_decode_for_reopened_wwise_banks_is_not_cached() {
        let mut audio = AudioState {
            engine_tried: true,
            wwise_generation: 3,
            ..Default::default()
        };
        audio.apply_job(decoded(
            0,
            Some(PcmKey::Event {
                generation: 2,
                name: "rifle_fire".to_owned(),
            }),
        ));
        assert!(audio.event_cache.get(&"rifle_fire".to_owned()).is_none());
    }

    fn missing_bank_fixture(name: &str) -> (PathBuf, ExtractRequest, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "baboon-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let tags = root.join("tags");
        std::fs::create_dir_all(&tags).unwrap();
        let output = root.join("data/sound/test.wav");
        let request = ExtractRequest {
            items: vec![super::super::sound_extract::ExtractItem {
                out_path: output.clone(),
                source: ExtractSource::Bank {
                    id: Some(123),
                    key: "test".to_owned(),
                    language: None,
                },
            }],
            tags_root: Some(tags),
            label: "test sound".to_owned(),
        };
        (root, request, output)
    }

    #[test]
    fn extraction_without_fmod_banks_is_cancelled_before_writing() {
        let (root, request, output) = missing_bank_fixture("missing-extract-bank");
        let mut audio = AudioState::default();

        audio.run_extract(request, &egui::Context::default());

        assert!(
            audio
                .status
                .as_deref()
                .is_some_and(|status| status.starts_with("Extraction cancelled — FMOD banks"))
        );
        assert!(!output.exists());
        assert_eq!(
            audio.jobs.running, 0,
            "a cancelled extraction starts no worker"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn player_reports_the_missing_fmod_bank_location() {
        let (root, request, _) = missing_bank_fixture("missing-player-bank");
        let tags = request.tags_root.unwrap();
        let mut audio = AudioState {
            pending: queued([SoundAction::Play {
                id: Some(123),
                key: "test".to_owned(),
                label: "test".to_owned(),
                tags_root: None,
            }]),
            ..Default::default()
        };

        audio.process(Some(&tags), &egui::Context::default());

        let status = audio.status.as_deref().expect("visible player error");
        assert!(status.starts_with("FMOD audio unavailable:"));
        let expected = root.join("fmod").join("pc").display().to_string();
        assert!(
            status.contains(&expected),
            "{status:?} did not contain {expected:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn player_uses_the_tags_own_kit_instead_of_the_active_kit() {
        let (root, request, _) = missing_bank_fixture("player-owning-kit");
        let owning_tags = request.tags_root.unwrap();
        let other_tags = root.join("other-kit/tags");
        std::fs::create_dir_all(&other_tags).unwrap();
        let mut audio = AudioState {
            pending: queued([SoundAction::Play {
                id: Some(123),
                key: "test".to_owned(),
                label: "test".to_owned(),
                tags_root: Some(owning_tags),
            }]),
            ..Default::default()
        };

        audio.process(Some(&other_tags), &egui::Context::default());

        let status = audio.status.as_deref().expect("visible player error");
        let expected = root.join("fmod").join("pc").display().to_string();
        let wrong = root
            .join("other-kit")
            .join("fmod")
            .join("pc")
            .display()
            .to_string();
        assert!(
            status.contains(&expected),
            "{status:?} did not contain {expected:?}"
        );
        assert!(
            !status.contains(&wrong),
            "player used the active kit: {status:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn language_fallback_does_not_overwrite_the_play_request() {
        let (root, request, _) = missing_bank_fixture("language-then-play");
        let tags = request.tags_root.unwrap();
        let play = SoundAction::Play {
            id: Some(123),
            key: "test".to_owned(),
            label: "test".to_owned(),
            tags_root: Some(tags.clone()),
        };
        let mut audio = AudioState {
            language: Some("language-from-another-kit".to_owned()),
            pending: queued([SoundAction::SetLanguage(None), play]),
            ..Default::default()
        };

        audio.process(Some(&tags), &egui::Context::default());
        assert_eq!(audio.language, None);
        assert_eq!(audio.pending.len(), 1, "Play was lost behind SetLanguage");

        audio.process(Some(&tags), &egui::Context::default());
        assert!(
            audio
                .status
                .as_deref()
                .is_some_and(|status| status.starts_with("FMOD audio unavailable:")),
            "the preserved Play action was not processed: {:?}",
            audio.status
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn player_retries_a_failed_fmod_open_without_a_restart() {
        let (root, request, _) = missing_bank_fixture("retry-player-bank");
        let tags = request.tags_root.unwrap();
        let play = || SoundAction::Play {
            id: Some(123),
            key: "test".to_owned(),
            label: "test".to_owned(),
            tags_root: None,
        };
        let mut audio = AudioState {
            pending: queued([play()]),
            ..Default::default()
        };
        audio.process(Some(&tags), &egui::Context::default());
        let first = audio.status.clone().expect("first bank error");

        let bank_dir = root.join("fmod").join("pc");
        std::fs::create_dir_all(&bank_dir).unwrap();
        std::fs::write(bank_dir.join("sfx.fsb"), b"new but invalid").unwrap();
        audio.pending.push_back(play().into());
        audio.process(Some(&tags), &egui::Context::default());
        let second = audio.status.clone().expect("retried bank error");

        assert_ne!(first, second, "the original missing-bank result was cached");
        assert!(second.contains("read header"), "{second}");
        let _ = std::fs::remove_dir_all(root);
    }

    fn owner(kit: u64, key: &str) -> SoundOwner {
        SoundOwner {
            kit: KitId(kit),
            key: key.to_owned(),
        }
    }

    /// Two-channel frames whose samples say where they came from:
    /// frame `f` holds `(f * 10, f * 10 + 1)`.
    fn numbered(frames: i16) -> Arc<DecodedPcm> {
        Arc::new(DecodedPcm {
            samples: (0..frames)
                .flat_map(|f| [f.wrapping_mul(10), f.wrapping_mul(10).wrapping_add(1)])
                .collect(),
            channels: 2,
            sample_rate: 1000,
        })
    }

    fn wave(frames: i16) -> Arc<Waveform> {
        Arc::new(Waveform::new(numbered(frames)))
    }

    fn pcm_source(pcm: &Arc<DecodedPcm>, looping: bool) -> (PcmSource, Arc<PlaybackShared>) {
        let shared = Arc::new(PlaybackShared::new(looping));
        let source = PcmSource {
            pcm: pcm.clone(),
            shared: shared.clone(),
            channels: 2,
            frames: pcm.frame_count() as u64,
            frame: 0,
            channel: 0,
        };
        (source, shared)
    }

    /// The source plays the shared buffer in order, reports where it is a
    /// whole frame at a time, takes a seek at the next frame boundary, and
    /// either ends or wraps at the end.
    #[test]
    fn the_pcm_source_reports_position_seeks_and_loops() {
        let pcm = numbered(4);
        let (mut source, shared) = pcm_source(&pcm, false);
        assert_eq!(source.next(), Some(0));
        assert_eq!(
            shared.frame.load(Ordering::Relaxed),
            0,
            "half a frame is not a frame"
        );
        assert_eq!(source.next(), Some(1));
        assert_eq!(shared.frame.load(Ordering::Relaxed), 1);
        shared.seek.store(3, Ordering::Relaxed);
        assert_eq!(source.next(), Some(30), "the seek lands on the next frame");
        assert_eq!(source.next(), Some(31));
        assert_eq!(source.next(), None, "no loop: the sound ends");
        assert_eq!(shared.frame.load(Ordering::Relaxed), 4);

        let (mut source, shared) = pcm_source(&pcm, true);
        shared.seek.store(3, Ordering::Relaxed);
        let played: Vec<i16> = source.by_ref().take(4).collect();
        assert_eq!(played, [30, 31, 0, 1], "looping wraps to the start");
    }

    /// A voice with no sink — nothing is playing — still keeps a playhead the
    /// player can move and read.
    #[test]
    fn a_stopped_voice_keeps_a_playhead_the_player_can_move() {
        let voice = Voice::new(wave(1000), "x".to_owned(), None, None, false);
        voice.seek(250);
        let view = voice.view();
        assert_eq!(view.position, 0.25);
        assert_eq!(view.duration, 1.0);
        assert!(!view.playing);
        voice.seek(5000);
        assert_eq!(
            voice.view().position,
            1.0,
            "a seek past the end stops at the end"
        );
    }

    /// Closing a tab disposes of its sound, and of a decode or Wwise load
    /// still on its way for it. Another tab closing leaves it alone.
    #[test]
    fn closing_its_tab_disposes_of_the_sound_and_what_is_coming_for_it() {
        let a = owner(1, "file:a.sound");
        let mut audio = AudioState {
            voice: Some(Voice::new(
                wave(10),
                "a".to_owned(),
                Some(a.clone()),
                None,
                false,
            )),
            decode_owner: Some(a.clone()),
            wwise_deferred: Some(("event".to_owned(), "a".to_owned(), Some(a.clone()), None)),
            play_request: 7,
            ..Default::default()
        };

        audio.follow_tabs(None, |owner| owner.key != "file:b.sound");
        assert!(audio.voice.is_some() && audio.wwise_deferred.is_some());
        assert_eq!(audio.play_request, 7, "another tab closing cancels nothing");

        audio.follow_tabs(None, |owner| owner.key != "file:a.sound");
        assert!(
            audio.voice.is_none(),
            "the closed tab's sound is disposed of"
        );
        assert!(audio.wwise_deferred.is_none(), "its Wwise play is dropped");
        assert_eq!(
            audio.play_request, 8,
            "its decode will not play when it lands"
        );
        assert!(audio.playback(Some(&a)).is_none());
    }

    /// The transport in one tab does not reach another tab's sound, and each
    /// tab sees only its own.
    #[test]
    fn transport_from_another_tab_does_not_move_this_tab_s_sound() {
        let a = owner(1, "file:a.sound");
        let b = owner(1, "file:b.sound");
        let mut audio = AudioState {
            voice: Some(Voice::new(
                wave(1000),
                "a".to_owned(),
                Some(a.clone()),
                None,
                false,
            )),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        audio.pending.push_back(SoundRequest {
            owner: Some(b.clone()),
            clip: None,
            preview: false,
            action: SoundAction::Seek(0.5),
        });
        audio.process(None, &ctx);
        assert_eq!(audio.playback(Some(&a)).unwrap().position, 0.0);
        assert!(audio.playback(Some(&b)).is_none());

        audio.pending.push_back(SoundRequest {
            owner: Some(a.clone()),
            clip: None,
            preview: false,
            action: SoundAction::Seek(0.5),
        });
        audio.process(None, &ctx);
        assert_eq!(audio.playback(Some(&a)).unwrap().position, 0.5);
    }

    /// Focus moving to another tab pauses the sound; coming back does not
    /// start it again. Needs an output device, so it skips without one.
    #[test]
    fn another_tab_taking_focus_pauses_the_sound() {
        let a = owner(1, "file:a.sound");
        let b = owner(1, "file:b.sound");
        let mut audio = AudioState::default();
        if audio.ensure_engine().is_none() {
            eprintln!("skipping: no audio output device");
            return;
        }
        audio.volume = Volume(0.0);
        audio.play_decoded(wave(30_000), "a", Some(a.clone()), None);
        assert!(audio.playback(Some(&a)).unwrap().playing);

        audio.follow_tabs(Some(&a), |_| true);
        assert!(
            audio.playback(Some(&a)).unwrap().playing,
            "its own tab keeps it playing"
        );

        audio.follow_tabs(Some(&b), |_| true);
        assert!(
            !audio.playback(Some(&a)).unwrap().playing,
            "another tab's focus pauses it"
        );

        audio.follow_tabs(Some(&a), |_| true);
        assert!(
            !audio.playback(Some(&a)).unwrap().playing,
            "returning does not resume it"
        );

        audio.pending.push_back(SoundRequest {
            owner: Some(a.clone()),
            clip: None,
            preview: false,
            action: SoundAction::TogglePause,
        });
        audio.process(None, &egui::Context::default());
        assert!(audio.playback(Some(&a)).unwrap().playing, "play resumes it");
    }

    /// A status line belongs to the tab whose sound caused it: a failure in
    /// one tab (or kit) does not show in another's player, a status no tab
    /// caused shows in all of them, and closing the tab clears its own.
    #[test]
    fn a_status_line_shows_only_in_the_tab_that_caused_it() {
        let a = owner(1, "file:a.sound");
        let b = owner(2, "file:b.sound");
        let mut audio = AudioState::default();
        audio.pending.push_back(SoundRequest {
            owner: Some(a.clone()),
            clip: None,
            preview: false,
            action: SoundAction::Play {
                id: None,
                key: "dth1".to_owned(),
                label: "dth1".to_owned(),
                tags_root: None,
            },
        });
        audio.process(None, &egui::Context::default());
        assert_eq!(audio.status.as_deref(), Some("no source loaded"));
        assert!(audio.status_is_for(&a));
        assert!(
            !audio.status_is_for(&b),
            "another kit's tab shows a's failure"
        );

        // Another tab's volume change leaves a's status a's.
        audio.pending.push_back(SoundRequest {
            owner: Some(b.clone()),
            clip: None,
            preview: false,
            action: SoundAction::SetVolume(0.5),
        });
        audio.process(None, &egui::Context::default());
        assert!(audio.status_is_for(&a) && !audio.status_is_for(&b));

        audio.follow_tabs(None, |owner| owner != &a);
        assert!(audio.status.is_none(), "closing a left its status behind");

        audio.apply_job(AudioDone::Extracted("extracted 3 file(s)".to_owned()));
        assert!(audio.status_is_for(&a) && audio.status_is_for(&b));
    }

    fn inline_pcm(frames: usize) -> SoundAction {
        SoundAction::PlayInline {
            bytes: (0..frames * 2)
                .flat_map(|i| ((i % 100) as i16 * 100).to_le_bytes())
                .collect(),
            codec: InlineCodec::Pcm { big_endian: false },
            channels: 2,
            sample_rate: 1000,
            chunk_offsets: Vec::new(),
            label: "pcm".to_owned(),
        }
    }

    fn preview_of(owner: &SoundOwner, clip: &str, action: SoundAction) -> SoundRequest {
        SoundRequest {
            owner: Some(owner.clone()),
            clip: Some(clip.to_owned()),
            preview: true,
            action,
        }
    }

    /// A preview decodes its clip for the waveform without touching playback:
    /// another tab's sound, the status line, a play still decoding and a
    /// deferred Wwise play are all left alone. Playing the clip afterwards
    /// starts no second decode.
    #[test]
    fn a_preview_decodes_without_disturbing_playback() {
        let a = owner(1, "file:a.sound");
        let b = owner(1, "file:b.sound");
        let mut audio = AudioState {
            voice: Some(Voice::new(
                wave(100),
                "a".to_owned(),
                Some(a.clone()),
                None,
                false,
            )),
            status: Some("\u{25B6} a".to_owned()),
            status_owner: Some(a.clone()),
            wwise_deferred: Some(("event".to_owned(), "e".to_owned(), Some(a.clone()), None)),
            play_request: 5,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        audio
            .pending
            .push_back(preview_of(&b, "clip", inline_pcm(1500)));
        audio.process(None, &ctx);
        assert!(matches!(
            audio.preview_for(&b).unwrap().state,
            PreviewState::Pending
        ));
        audio.wait_for_audio_jobs();
        let PreviewState::Ready(waveform) = &audio.preview_for(&b).unwrap().state else {
            panic!("the preview did not land");
        };
        assert_eq!((waveform.channels(), waveform.frames()), (2, 1500));
        assert_eq!(audio.status.as_deref(), Some("\u{25B6} a"));
        assert!(audio.status_is_for(&a) && !audio.status_is_for(&b));
        assert_eq!(audio.play_request, 5, "the preview superseded a play");
        assert!(
            audio.wwise_deferred.is_some(),
            "the preview dropped a deferred play"
        );
        assert_eq!(
            audio.voice.as_ref().and_then(|voice| voice.owner.clone()),
            Some(a.clone())
        );

        // Playing the previewed clip uses the preview: no decode starts.
        audio.pending.push_back(SoundRequest {
            owner: Some(b.clone()),
            clip: Some("clip".to_owned()),
            preview: false,
            action: inline_pcm(1500),
        });
        audio.process(None, &ctx);
        assert_eq!(
            audio.jobs.running, 0,
            "the previewed clip was decoded again"
        );

        audio.follow_tabs(None, |owner| owner != &b);
        assert!(
            audio.preview_for(&b).is_none(),
            "closing b kept its preview"
        );
    }

    /// A preview with nothing to show says why, in the preview rather than
    /// on the status line.
    #[test]
    fn a_preview_that_resolves_nothing_says_why() {
        let b = owner(1, "file:b.sound");
        let mut audio = AudioState::default();
        audio.pending.push_back(preview_of(
            &b,
            "clip",
            SoundAction::Play {
                id: None,
                key: "k".to_owned(),
                label: "k".to_owned(),
                tags_root: None,
            },
        ));
        audio.process(None, &egui::Context::default());
        let PreviewState::Failed(reason) = &audio.preview_for(&b).unwrap().state else {
            panic!("no failure recorded");
        };
        assert_eq!(reason, "no source loaded");
        assert!(audio.status.is_none(), "a preview wrote the status line");
    }

    /// A region plays its frames and stops at its end, or wraps to its start
    /// when looping.
    #[test]
    fn the_source_plays_a_region_and_loops_it() {
        let pcm = numbered(10);
        let (mut source, shared) = pcm_source(&pcm, false);
        shared.region_start.store(3, Ordering::Relaxed);
        shared.region_end.store(5, Ordering::Relaxed);
        shared.seek.store(3, Ordering::Relaxed);
        let played: Vec<i16> = source.by_ref().collect();
        assert_eq!(played, [30, 31, 40, 41], "the region, then stop");
        assert_eq!(shared.frame.load(Ordering::Relaxed), 5);

        let (mut source, shared) = pcm_source(&pcm, true);
        shared.region_start.store(3, Ordering::Relaxed);
        shared.region_end.store(5, Ordering::Relaxed);
        shared.seek.store(4, Ordering::Relaxed);
        let played: Vec<i16> = source.by_ref().take(6).collect();
        assert_eq!(played, [40, 41, 30, 31, 40, 41], "the region, looped");
    }

    #[test]
    fn play_starts_at_the_playhead_inside_the_region_else_its_start() {
        assert_eq!(play_from(4, (3, 8)), 4);
        assert_eq!(play_from(1, (3, 8)), 3, "before the region");
        assert_eq!(play_from(8, (3, 8)), 3, "at its end: again from the start");
        assert_eq!(
            play_from(10, (0, 10)),
            0,
            "no region, finished: from the top"
        );
    }

    /// A region set on a clip before it is loaded holds when it plays; one
    /// set on another clip does not. Needs an output device.
    #[test]
    fn a_region_set_before_loading_holds_for_its_clip() {
        let a = owner(1, "file:a.sound");
        let mut audio = AudioState::default();
        if audio.ensure_engine().is_none() {
            eprintln!("skipping: no audio output device");
            return;
        }
        audio.volume = Volume(0.0);
        audio.pending.push_back(SoundRequest {
            owner: Some(a.clone()),
            clip: Some("c".to_owned()),
            preview: false,
            action: SoundAction::SetRegion(Some((0.25, 0.5))),
        });
        audio.process(None, &egui::Context::default());
        audio.play_decoded(wave(1000), "c", Some(a.clone()), Some("c".to_owned()));
        assert_eq!(audio.voice.as_ref().unwrap().region(), Some((0.25, 0.5)));
        assert!(
            audio.playback(Some(&a)).unwrap().position >= 0.25,
            "play did not start at the region"
        );

        audio.play_decoded(wave(1000), "d", Some(a.clone()), Some("d".to_owned()));
        assert_eq!(
            audio.voice.as_ref().unwrap().region(),
            None,
            "another clip took the region"
        );
    }
}
