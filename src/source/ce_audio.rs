//! Campaign Evolved sound-tag → Wwise media resolution.
//!
//! Every earlier Blam engine stores sample data in the `sound` tag itself. CE
//! does not: its `sound` tag is metadata only, and the audio lives in Wwise. The
//! binding is expressed as UE package imports out of the tag's cooked `.uasset`
//! wrapper:
//!
//! ```text
//! /Game/Tags/sound/<path>-sound
//!   -> /Game/Audio/<path>/<name>_player_variant     (BlamAudioSoundCombiner)
//!      -> .../<name>_player | <name>_non-player     (BlamAudioSound)
//!         -> /Game/Wwise/Events/Play_<event>        (SFX)
//!         -> /Game/WwiseAudio/Events/.../Play_<ev>  (systemic voice)
//! ```
//!
//! The terminal `AkAudioEvent` names the `.wem` media per language. That media
//! is *not* in IoStore — it is staged loose in the legacy `.pak` containers —
//! so playback needs both indexes: [`ContainerPackageIndex`] to walk the
//! imports and a `PakSet` to fetch the bytes.

use std::collections::{BTreeSet, VecDeque};
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use blam_tags::iostore::container_header::EIoContainerHeaderVersion;
use blam_tags::iostore::pak::PakSet;
use blam_tags::iostore::ue_types::EIoStoreTocVersion;
use blam_tags::iostore::usmap::Usmap;
use blam_tags::iostore::wwise_event::{EventCookedData, read_event_cooked_data};
use blam_tags::iostore::zen::FZenPackageHeader;

use super::{ContainerPackageIndex, MountedContainer};

/// Container versions the CE build is cooked with.
const TOC_VERSION: EIoStoreTocVersion = EIoStoreTocVersion::ReplaceIoChunkHashWithIoHash;
const HEADER_VERSION: EIoContainerHeaderVersion = EIoContainerHeaderVersion::SoftPackageReferences;

/// Where Wwise media is mounted inside the legacy `.pak` set. Media paths in
/// the cooked event data are relative to this.
const WWISE_MOUNT: &str = "Meteorite/Content/WwiseAudio";

/// Guard against a pathological import graph; the real chains are 3–4 deep.
const MAX_PACKAGE_VISITS: usize = 512;

/// One playable permutation resolved from a CE sound tag.
#[derive(Clone, Debug)]
pub struct CeSoundMedia {
    /// Wwise event that plays it, e.g. `Play_Foo_Bar`.
    pub event_name: String,
    /// Wwise language: `SFX` for non-localized, else e.g. `English(US)`.
    pub language: String,
    /// Wwise short id (also the file stem).
    pub media_id: u32,
    /// Path within the pak set's Wwise mount, e.g. `Media/43/43030714.wem`.
    pub media_path: String,
    /// Authoring `.wav` name — the closest thing to a permutation name.
    pub source_name: String,
}

impl CeSoundMedia {
    /// Path to look up in the pak set.
    pub fn mounted_path(&self) -> String {
        format!("{WWISE_MOUNT}/{}", self.media_path)
    }

    /// Short label for the UI: the source `.wav` stem when known, else the id.
    pub fn display_name(&self) -> String {
        let stem = self
            .source_name
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&self.source_name)
            .trim_end_matches(".wav");
        if stem.is_empty() { self.media_id.to_string() } else { stem.to_string() }
    }
}

/// Everything a CE sound tag binds to, grouped by the events it reaches.
#[derive(Clone, Debug, Default)]
pub struct CeSoundBinding {
    pub events: Vec<EventCookedData>,
    pub media: Vec<CeSoundMedia>,
}

impl CeSoundBinding {
    pub fn is_empty(&self) -> bool {
        self.media.is_empty()
    }

    /// Distinct languages across all media, in first-seen order, `SFX` first.
    pub fn languages(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for m in &self.media {
            if !out.iter().any(|l| l.eq_ignore_ascii_case(&m.language)) {
                out.push(m.language.clone());
            }
        }
        out.sort_by_key(|l| !l.eq_ignore_ascii_case("SFX"));
        out
    }

    /// Media for one language, falling back to `SFX` (non-localized events
    /// carry only `SFX` no matter which language the user picked).
    pub fn media_for_language(&self, language: &str) -> Vec<&CeSoundMedia> {
        let exact: Vec<&CeSoundMedia> =
            self.media.iter().filter(|m| m.language.eq_ignore_ascii_case(language)).collect();
        if !exact.is_empty() {
            return exact;
        }
        self.media.iter().filter(|m| m.language.eq_ignore_ascii_case("SFX")).collect()
    }
}

/// The cooked package name for a mounted tag payload.
///
/// A tag's browser entry points at its `.ubulk` (the Reach tag bytes); the
/// import graph lives in the sibling `.uasset` wrapper, so swap the extension
/// before looking the package up.
pub fn tag_package_for_rel_path(rel_path: &str) -> Option<String> {
    let base = rel_path.strip_suffix(".ubulk").or_else(|| rel_path.strip_suffix(".uasset"))?;
    super::container_package_name(&format!("{base}.uasset"))
}

/// Read a cooked package's Zen header plus its first export's serial range.
fn read_package(
    containers: &[MountedContainer],
    packages: &ContainerPackageIndex,
    package: &str,
) -> Option<(FZenPackageHeader, Vec<u8>)> {
    let (container, rel) = packages.lookup(package)?;
    let archive = &containers.get(container)?.archive;
    let bytes = archive.read(rel).ok()?;
    let header =
        FZenPackageHeader::deserialize(&mut Cursor::new(&bytes), None, TOC_VERSION, HEADER_VERSION, None)
            .ok()?;
    Some((header, bytes))
}

/// Whether a package name is one of the two Wwise event roots. SFX events live
/// under `/Game/Wwise/Events`, systemic voice under `/Game/WwiseAudio/Events`;
/// missing the second silently resolves nothing for all dialogue.
fn is_event_package(lower: &str) -> bool {
    lower.starts_with("/game/wwise/events/") || lower.starts_with("/game/wwiseaudio/events/")
}

/// Walk package imports from a CE sound tag to the Wwise media it plays.
///
/// `tag_package` is the tag's cooked package name (`/Game/Tags/sound/...`).
/// Returns an empty binding for the many `sound` tags that are unbound stubs
/// with no imports at all — that is a normal state, not an error.
pub fn resolve_sound_binding(
    containers: &[MountedContainer],
    packages: &ContainerPackageIndex,
    usmap: &Usmap,
    tag_package: &str,
) -> CeSoundBinding {
    let mut binding = CeSoundBinding::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut event_packages: Vec<String> = Vec::new();

    seen.insert(tag_package.to_ascii_lowercase());
    queue.push_back(tag_package.to_string());
    let mut visits = 0usize;

    while let Some(package) = queue.pop_front() {
        visits += 1;
        if visits > MAX_PACKAGE_VISITS {
            break;
        }
        let Some((header, _)) = read_package(containers, packages, &package) else { continue };
        for import in &header.imported_package_names {
            let lower = import.to_ascii_lowercase();
            if !seen.insert(lower.clone()) {
                continue;
            }
            if is_event_package(&lower) {
                event_packages.push(import.clone());
            } else if lower.starts_with("/game/audio/") {
                queue.push_back(import.clone());
            }
        }
    }

    for package in &event_packages {
        let Some((header, bytes)) = read_package(containers, packages, package) else { continue };
        let Some(export) = header.export_map.first() else { continue };
        let start = header.summary.header_size as usize + export.cooked_serial_offset as usize;
        let end = start + export.cooked_serial_size as usize;
        let Some(body) = bytes.get(start..end) else { continue };
        let names = header.name_map.copy_raw_names();
        let Ok(cooked) = read_event_cooked_data(body, &names, usmap) else { continue };

        for m in &cooked.media {
            binding.media.push(CeSoundMedia {
                event_name: cooked.event_name.clone(),
                language: m.language.clone(),
                media_id: m.media_id,
                media_path: m.path.clone(),
                source_name: m.source_name.clone(),
            });
        }
        binding.events.push(cooked);
    }
    binding
}

/// The legacy `.pak` set holding Wwise media, opened lazily.
///
/// Opening every container parses each one's directory index, so this is done
/// once on first playback rather than at mount — a source can be browsed and
/// edited without ever touching audio.
#[derive(Default)]
pub struct CeMediaStore {
    paks: Option<PakSet>,
}

impl CeMediaStore {
    /// Open the pak set rooted at the source's `Paks` directory, if not already.
    fn paks(&mut self, paks_root: &Path) -> Result<&mut PakSet> {
        if self.paks.is_none() {
            let set = PakSet::open_dir(paks_root)
                .with_context(|| format!("opening pak set at {}", paks_root.display()))?;
            if set.is_empty() {
                return Err(anyhow!("no readable .pak containers in {}", paks_root.display()));
            }
            self.paks = Some(set);
        }
        Ok(self.paks.as_mut().expect("just populated"))
    }

    /// Fetch and decode one media entry to PCM.
    pub fn decode(
        &mut self,
        paks_root: &Path,
        media: &CeSoundMedia,
    ) -> Result<blam_tags::audio::DecodedPcm> {
        let path = media.mounted_path();
        let bytes = self
            .paks(paks_root)?
            .read(&path)
            .with_context(|| format!("reading {path} from the pak set"))?;
        blam_tags::audio::wwise::decode_wem(&bytes)
            .map_err(|e| anyhow!("decoding {path}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounted_path_prefixes_the_wwise_mount() {
        let m = CeSoundMedia {
            event_name: "Play_X".into(),
            language: "SFX".into(),
            media_id: 43030714,
            media_path: "Media/43/43030714.wem".into(),
            source_name: String::new(),
        };
        assert_eq!(m.mounted_path(), "Meteorite/Content/WwiseAudio/Media/43/43030714.wem");
        // With no source name, fall back to the id rather than an empty label.
        assert_eq!(m.display_name(), "43030714");
    }

    #[test]
    fn display_name_uses_the_authoring_wav_stem() {
        let m = CeSoundMedia {
            event_name: "Play_X".into(),
            language: "SFX".into(),
            media_id: 1,
            media_path: "Media/1/1.wem".into(),
            source_name: r"000_Olympus\006_Character\Foo_AR_ReloadMagHit_02.wav".into(),
        };
        assert_eq!(m.display_name(), "Foo_AR_ReloadMagHit_02");
    }

    #[test]
    fn localized_lookup_falls_back_to_sfx() {
        let sfx = CeSoundMedia {
            event_name: "Play_X".into(),
            language: "SFX".into(),
            media_id: 1,
            media_path: "Media/1/1.wem".into(),
            source_name: String::new(),
        };
        let binding = CeSoundBinding { events: Vec::new(), media: vec![sfx] };
        // A non-localized event has no English(US) entry; asking for one must
        // still play rather than silently returning nothing.
        assert_eq!(binding.media_for_language("English(US)").len(), 1);
        assert_eq!(binding.languages(), vec!["SFX".to_string()]);
    }

    #[test]
    fn both_event_roots_are_recognized() {
        assert!(is_event_package("/game/wwise/events/play_foo"));
        assert!(is_event_package("/game/wwiseaudio/events/systemic/vo/play_bar"));
        assert!(!is_event_package("/game/audio/systemic/vo_thing"));
    }

    /// End-to-end against a real Campaign Evolved install: mount the
    /// containers, walk a known sound tag's imports, and decode the media it
    /// resolves to. Ignored by default — it needs the shipped game.
    ///
    /// Run with:
    ///   CE_PAKS=/path/to/Meteorite/Content/Paks cargo test ce_audio -- --ignored --nocapture
    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn resolves_and_decodes_real_sound_tags() {
        use blam_tags::iostore::IoStoreArchive;
        use std::path::PathBuf;
        use std::sync::Arc;

        let root = PathBuf::from(
            std::env::var("CE_PAKS").expect("set CE_PAKS to the game's Content/Paks"),
        );

        let mut utocs: Vec<PathBuf> = std::fs::read_dir(&root)
            .expect("read paks dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("utoc")))
            .filter(|p| !p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("global.utoc")))
            .collect();
        utocs.sort();

        let mut containers = Vec::new();
        let mut packages = ContainerPackageIndex::default();
        for utoc in utocs {
            let Ok(archive) = IoStoreArchive::open(&utoc) else { continue };
            let idx = containers.len();
            for e in archive.entries() {
                if let Some(pkg) = super::super::container_package_name(&e.path) {
                    packages.insert(pkg, idx, e.path.clone());
                }
            }
            containers.push(MountedContainer {
                utoc_path: utoc.clone(),
                chunk_label: utoc.file_stem().unwrap().to_string_lossy().into_owned(),
                archive: Arc::new(archive),
            });
        }
        assert!(!packages.is_empty(), "no cooked packages indexed");
        println!("indexed {} packages across {} containers", packages.len(), containers.len());

        // The controller reaches a tag by its browser entry, which points at the
        // `.ubulk` payload — so the `.ubulk` → package mapping must land on a
        // package that actually exists. Check it over every mounted sound tag.
        let mut sound_tags = 0usize;
        let mut resolved = 0usize;
        for c in &containers {
            for e in c.archive.ublock_entries() {
                if !e.path.to_ascii_lowercase().ends_with("-sound.ubulk") {
                    continue;
                }
                sound_tags += 1;
                if tag_package_for_rel_path(&e.path)
                    .is_some_and(|pkg| packages.lookup(&pkg).is_some())
                {
                    resolved += 1;
                }
            }
        }
        println!("sound tags: {sound_tags}, package mapping resolved: {resolved}");
        assert!(sound_tags > 0, "no sound tags mounted");
        assert_eq!(sound_tags, resolved, "some sound tags had no cooked package");

        let usmap = Usmap::meteorite().expect("bundled usmap");
        let mut store = CeMediaStore::default();

        // A non-localized SFX tag and a localized dialogue tag: the two shapes
        // that reach different event roots.
        let cases = [
            "/Game/Tags/sound/005_sandbox/006_character/006_character_movement/\
             006_chm_ge_weaanim/006_chm_ge_weaanim_ar_reloadmaghit-sound",
            "/Game/Tags/sound/dialog/combat/bisenti/default/01_contact/ambush-sound",
        ];

        for tag in cases {
            let tag = tag.replace(char::is_whitespace, "");
            let binding = resolve_sound_binding(&containers, &packages, &usmap, &tag);
            assert!(!binding.is_empty(), "no media resolved for {tag}");
            println!(
                "{tag}\n  events={} media={} languages={:?}",
                binding.events.len(),
                binding.media.len(),
                binding.languages()
            );

            // Every event's id must equal the Wwise hash of its own name — a
            // misread of the cooked struct cannot satisfy that.
            for ev in &binding.events {
                assert_eq!(
                    blam_tags::audio::wwise::hash_name(&ev.event_name),
                    ev.event_id,
                    "event id/name mismatch for {}",
                    ev.event_name
                );
            }

            // And the media has to actually decode to non-silent audio.
            for m in binding.media_for_language("English(US)") {
                let pcm = store.decode(&root, m).expect("decode media");
                assert!(!pcm.samples.is_empty(), "{} decoded to nothing", m.media_path);
                let peak = pcm.samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
                assert!(peak > 0, "{} decoded to silence", m.media_path);
                println!(
                    "  {} [{}] {} ch {} Hz peak {peak}",
                    m.display_name(),
                    m.language,
                    pcm.channels,
                    pcm.sample_rate
                );
            }
        }
    }
}
