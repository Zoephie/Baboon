//! Sound-tag audio *extraction*: dump a `.sound` tag's permutations to disk in
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.
//! a layout the game's `tool.exe` can reimport, closing the audition→edit→
//! reimport loop. No tool has a first-class audio export verb, so this fills
//! that gap by reusing the same decoders the audition path uses.
//!
//! Fidelity depends on the game (see the module notes in `audio.rs`):
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

/// One file to write during an extraction.
pub(super) struct ExtractItem {
    pub(super) out_path: PathBuf,
    pub(super) source: ExtractSource,
}

/// Where an item's audio comes from and how to turn it into a file.
pub(super) enum ExtractSource {
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
    Bank { id: Option<u32>, key: String },
    /// Resolve a Wwise event by name (H4), decode, write WAV.
    Event { name: String },
}

/// A batch of files to extract, queued by the sound-player UI and drained by
/// the audio layer.
pub(super) struct ExtractRequest {
    pub(super) items: Vec<ExtractItem>,
    /// Tags root of the current source, needed to open FMOD/Wwise banks.
    pub(super) tags_root: Option<PathBuf>,
    /// Human label for the resulting status line (tag or permutation name).
    pub(super) label: String,
}

/// Turn a filesystem-unsafe permutation/pitch-range string-id into a clean file
/// stem (tool names permutations by filename, so keep it faithful but legal).
pub(super) fn sanitize_component(name: &str) -> String {
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
    } else {
        trimmed.to_owned()
    }
}

/// The `data\` root for a source: sibling of the `tags\` root in an editing kit
/// The `data\` (default) or `data_<language>\` root beside the tags tree
/// (`<EK>/tags` → `<EK>/data[_<lang>]`) — the tool exports/imports non-default
/// languages from `data_<language>\`.
pub(super) fn data_root_for_language(tags_root: &Path, language: Option<&str>) -> Option<PathBuf> {
    let ek = tags_root.parent()?;
    Some(match language {
        Some(lang) => ek.join(format!("data_{lang}")),
        None => ek.join("data"),
    })
}

/// The reimport-layout base directory for a tag: `data[_<language>]\<tag path
/// minus extension>\`. `abs_tag_path` is the loose `.sound` file; `tags_root` its
/// root. `language = None` → `data\` (the default/primary language).
pub(super) fn reimport_base_dir_lang(
    tags_root: &Path,
    abs_tag_path: &Path,
    language: Option<&str>,
) -> Option<PathBuf> {
    let data_root = data_root_for_language(tags_root, language)?;
    let rel = abs_tag_path.strip_prefix(tags_root).ok()?;
    Some(data_root.join(rel.with_extension("")))
}

/// Write interleaved 16-bit PCM as a canonical little-endian WAV, creating
/// parent directories. Channel count and sample rate are preserved verbatim so
/// a reimport sees the original geometry.
pub(super) fn write_wav_pcm16(
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
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_canonical_pcm16() {
        let dir = std::env::temp_dir().join("baboon_wav_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("t.wav");
        write_wav_pcm16(&path, &[0, 1, -1, 32767], 2, 44_100).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1); // PCM
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2); // channels
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            44_100
        );
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16); // bits
        assert_eq!(&bytes[36..40], b"data");
        // 4 samples * 2 bytes = 8 bytes of data.
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            8
        );
        assert_eq!(bytes.len(), 44 + 8);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sanitize_strips_path_separators() {
        assert_eq!(sanitize_component("ambient/expl:1"), "ambient_expl_1");
        assert_eq!(sanitize_component("  "), "sound");
        assert_eq!(sanitize_component("plain_name"), "plain_name");
    }

    #[test]
    fn extract_base_dir_mirrors_the_tag() {
        let tags_root = Path::new("/ek/tags");
        let tag = Path::new("/ek/tags/sound/weapons/rifle.sound");
        assert_eq!(
            reimport_base_dir_lang(tags_root, tag, None).unwrap(),
            PathBuf::from("/ek/data/sound/weapons/rifle")
        );
        // A non-default language routes to `data_<lang>\`.
        assert_eq!(
            reimport_base_dir_lang(tags_root, tag, Some("french")).unwrap(),
            PathBuf::from("/ek/data_french/sound/weapons/rifle")
        );
    }
}
