//! Sound-tag audio *extraction*: dump a `.sound` tag's permutations to disk in
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.
//! a layout the game's `tool.exe` can reimport, closing the audition→edit→
//! reimport loop. No tool has a first-class audio export verb, so this fills
//! that gap by reusing the same decoders the audition path uses.
//!
//! Fidelity depends on the game (see the `audio` module notes):
//! - **CE** — the tag holds a *complete inline Ogg Vorbis* stream, so a raw
//!   passthrough (`.ogg`, verbatim bytes) is near-lossless; decoding to WAV
//!   instead forces `tool sounds ... ogg` to re-encode.
//! - **H2 PCM ("none")** — samples are uncompressed; WAV round-trips losslessly.
//! - **H2 opus/adpcm, H3/ODST/Reach** — decode→WAV→reimport re-encodes (same
//!   structure and perceptual audio, not byte-identical).
//! - **H4** — Wwise; extract-only (no `tool.exe` reimport path).
//!
//! The UI builds an [`ExtractRequest`] (resolving each permutation's bytes/key
//! up front); [`super::audio::AudioState::run_extract`] does the decode + write
//! off the render path's hot loop.

use std::path::{Path, PathBuf};

use super::audio::InlineCodec;
use crate::core::source::KitLayout;

/// One file to write during an extraction.
pub(in crate::app) struct ExtractItem {
    pub(in crate::app) out_path: PathBuf,
    pub(in crate::app) source: ExtractSource,
}

/// Where an item's audio comes from and how to turn it into a file.
pub(in crate::app) enum ExtractSource {
    /// Write these bytes verbatim (CE inline Ogg passthrough → near-lossless).
    Raw(Vec<u8>),
    /// Decode inline classic audio (CE/H2), then write 16-bit PCM WAV.
    /// `chunk_offsets` = H2 per-chunk byte offsets (empty = single stream / CE).
    Inline {
        bytes: Vec<u8>,
        codec: InlineCodec,
        channels: u16,
        sample_rate: u32,
        chunk_offsets: Vec<usize>,
    },
    /// Resolve an FMOD bank subsound (H3/ODST/Reach), decode, write WAV.
    /// `id` is the engine's `fmod bank subsound id hash` (preferred); `key` is
    /// the permutation leaf name (legacy fallback). See `AudioState::bank_pcm`.
    Bank {
        id: Option<u32>,
        key: String,
        language: Option<String>,
    },
    /// Resolve a Wwise event by name and language (H4), decode, write WAV.
    Event {
        name: String,
        language: Option<String>,
    },
    /// Decode one already-resolved Campaign Evolved Wwise media file, write
    /// WAV. Unlike [`ExtractSource::Event`] this needs no prior play: CE's
    /// media is addressed directly in the legacy `.pak` set rooted at
    /// `paks_root`, so nothing has to be indexed first.
    CeMedia {
        paks_root: PathBuf,
        media: Box<crate::core::source::ce_audio::CeSoundMedia>,
    },
}

/// A batch of files to extract, queued by the sound-player UI and drained by
/// the audio layer.
pub(in crate::app) struct ExtractRequest {
    pub(in crate::app) items: Vec<ExtractItem>,
    /// Tags root of the current source, needed to open FMOD/Wwise banks.
    pub(in crate::app) tags_root: Option<PathBuf>,
    /// Human label for the resulting status line (tag or permutation name).
    pub(in crate::app) label: String,
}

/// Turn a filesystem-unsafe permutation/pitch-range string-id into a clean file
/// stem (tool names permutations by filename, so keep it faithful but legal).
///
/// A name Windows reserves for a device (`con`, `nul`, `com1`, ...) gets a
/// leading underscore: as a file stem it names the device, so writing
/// `nul.wav` there writes nowhere and `con.wav` fails.
pub(in crate::app) fn sanitize_component(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "sound".to_owned()
    } else if crate::app::is_windows_reserved_name(trimmed) {
        format!("_{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

/// The reimport-layout base directory for a tag: `data[_<language>]\<tag path
/// minus extension>\`, in the kit's data folder for `language` (see
/// [`KitLayout::data_for_language`]); the tool exports and imports non-default
/// languages from `data_<language>\`. `abs_tag_path` is the loose `.sound` file,
/// which must be under the kit's tags folder.
pub(in crate::app) fn reimport_base_dir_lang(
    layout: &KitLayout,
    abs_tag_path: &Path,
    language: Option<&str>,
) -> Option<PathBuf> {
    let rel = abs_tag_path.strip_prefix(&layout.tags).ok()?;
    Some(
        layout
            .data_for_language(language)
            .join(rel.with_extension("")),
    )
}

/// Write interleaved 16-bit PCM as a canonical little-endian WAV, creating
/// parent directories. Channel count and sample rate are preserved verbatim so
/// a reimport sees the original geometry.
pub(in crate::app) fn write_wav_pcm16(
    path: &Path,
    samples: &[i16],
    channels: u16,
    sample_rate: u32,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let channels = channels.max(1);
    let block_align = u32::from(channels) * 2;
    let byte_rate = sample_rate * block_align;
    let data_len = (samples.len() * 2) as u32;
    let mut buf = Vec::with_capacity(44 + samples.len() * 2);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&(block_align as u16).to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        buf.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, buf)
}

#[cfg(test)]
mod tests;
