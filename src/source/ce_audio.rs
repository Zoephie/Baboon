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

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use blam_tags::audio::wwise::{Bnk, HircIndex, SoundSource};
use blam_tags::iostore::container_header::EIoContainerHeaderVersion;
use blam_tags::iostore::pak::PakSet;
use blam_tags::iostore::ue_types::{EIoStoreTocVersion, FPackageObjectIndex};
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

/// Where one permutation's bytes sit inside the pak set.
///
/// Roughly a tenth of the game's sounds are cooked with their media *inside* a
/// SoundBank rather than staged as loose `.wem` — for those, the event's cooked
/// data lists banks and no media at all, and the permutations only become
/// visible after reading the bank's own event graph.
#[derive(Clone, Debug)]
pub enum CeMediaLocation {
    /// A loose `.wem` under the Wwise mount, e.g. `Media/43/43030714.wem`.
    Loose(String),
    /// Embedded in a SoundBank's `DATA` chunk, e.g. `772982525.bnk`, keyed by
    /// [`CeSoundMedia::media_id`].
    Bank(String),
}

/// One playable permutation resolved from a CE sound tag.
#[derive(Clone, Debug)]
pub struct CeSoundMedia {
    /// Wwise event that plays it, e.g. `Play_Foo_Bar`.
    pub event_name: String,
    /// Wwise language: `SFX` for non-localized, else e.g. `English(US)`.
    pub language: String,
    /// Wwise short id (also the file stem, and the key into a bank's media).
    pub media_id: u32,
    /// Where the bytes live within the pak set's Wwise mount.
    pub location: CeMediaLocation,
    /// Authoring `.wav` name — the closest thing to a permutation name. Empty
    /// for bank-embedded media, which carries no debug name.
    pub source_name: String,
}

impl CeSoundMedia {
    /// Path to look up in the pak set — the `.wem` itself, or the bank holding it.
    pub fn mounted_path(&self) -> String {
        let rel = match &self.location {
            CeMediaLocation::Loose(path) | CeMediaLocation::Bank(path) => path,
        };
        format!("{WWISE_MOUNT}/{rel}")
    }

    /// Where the audio comes from, for the player's provenance column.
    pub fn location_label(&self) -> String {
        match &self.location {
            CeMediaLocation::Loose(path) => path.clone(),
            CeMediaLocation::Bank(bank) => format!("{bank} \u{2192} {}", self.media_id),
        }
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
    /// carry only `SFX` no matter which language the user picked) and then to
    /// everything. Never returns empty while the binding holds media — a tag
    /// that resolved audio must show rows for it.
    pub fn media_for_language(&self, language: &str) -> Vec<&CeSoundMedia> {
        let exact: Vec<&CeSoundMedia> =
            self.media.iter().filter(|m| m.language.eq_ignore_ascii_case(language)).collect();
        if !exact.is_empty() {
            return exact;
        }
        let sfx: Vec<&CeSoundMedia> =
            self.media.iter().filter(|m| m.language.eq_ignore_ascii_case("SFX")).collect();
        if !sfx.is_empty() {
            return sfx;
        }
        self.media.iter().collect()
    }

    /// Which language to show, given the player's current selection.
    ///
    /// `preferred` is only honoured when this tag actually carries it —
    /// localized voice has no `SFX` entry and non-localized audio has *only*
    /// `SFX`, so a selection made against one shape must not blank the other.
    /// With nothing selected, prefer English before falling back to whatever
    /// the event cooked (the list is otherwise alphabetical, which would
    /// arbitrarily land on Chinese).
    pub fn language_to_show(&self, preferred: Option<&str>) -> String {
        let languages = self.languages();
        let has = |name: &str| languages.iter().find(|l| l.eq_ignore_ascii_case(name)).cloned();

        preferred
            .and_then(has)
            .or_else(|| has("English(US)"))
            .or_else(|| has("English(UK)"))
            .or_else(|| languages.first().cloned())
            .unwrap_or_else(|| "SFX".to_string())
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

/// Whether a package is a Wwise event, decided by the class it exports rather
/// than where it sits.
///
/// Path prefixes do not work: events are spread over at least three roots
/// (`/Game/Wwise/Events`, `/Game/WwiseAudio/Events`, and `/Game/Audio/Audio_FI/…`
/// mixed in among the `BlamAudioSound` assets), and each root missed silently
/// unbinds every tag that reaches through it. The export class is the thing
/// that actually defines an event.
fn exports_ak_audio_event(header: &FZenPackageHeader) -> bool {
    header.exports_class(FPackageObjectIndex::create_script_import(AK_AUDIO_EVENT_CLASS))
}

/// The native `AkAudioEvent` UClass, as it appears in a cooked package's
/// `class_index` (a `ScriptImport` hash of this path).
const AK_AUDIO_EVENT_CLASS: &str = "/Script/AkAudio.AkAudioEvent";

/// Whether a package is worth walking into while looking for events. The audio
/// graph is confined to these roots; without a bound the walk would wander the
/// whole cooked package graph.
fn is_audio_package(lower: &str) -> bool {
    lower.starts_with("/game/audio/")
        || lower.starts_with("/game/wwise/")
        || lower.starts_with("/game/wwiseaudio/")
}

/// Walk package imports from a CE sound tag to the Wwise media it plays.
///
/// `tag_package` is the tag's cooked package name (`/Game/Tags/sound/...`).
/// Returns an empty binding for the many `sound` tags that are unbound stubs
/// with no imports at all — that is a normal state, not an error.
///
/// `banks` gives access to the pak set, needed only for events whose media is
/// cooked *inside* a SoundBank: those name no media in the package at all, so
/// the permutations have to be read out of the bank's own event graph. Pass
/// `None` to resolve package-staged media only.
pub fn resolve_sound_binding(
    containers: &[MountedContainer],
    packages: &ContainerPackageIndex,
    usmap: &Usmap,
    tag_package: &str,
    banks: Option<(&Path, &mut CeMediaStore)>,
) -> CeSoundBinding {
    let mut binding = CeSoundBinding::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut event_packages: Vec<(FZenPackageHeader, Vec<u8>)> = Vec::new();

    seen.insert(tag_package.to_ascii_lowercase());
    queue.push_back(tag_package.to_string());
    let mut visits = 0usize;

    while let Some(package) = queue.pop_front() {
        visits += 1;
        if visits > MAX_PACKAGE_VISITS {
            break;
        }
        let Some((header, bytes)) = read_package(containers, packages, &package) else { continue };
        // An event is a leaf: it holds the media, and nothing further to walk.
        if exports_ak_audio_event(&header) {
            event_packages.push((header, bytes));
            continue;
        }
        for import in &header.imported_package_names {
            let lower = import.to_ascii_lowercase();
            if !seen.insert(lower.clone()) {
                continue;
            }
            if is_audio_package(&lower) {
                queue.push_back(import.clone());
            }
        }
    }

    let mut banks = banks;
    for (header, bytes) in &event_packages {
        let Some(export) = header
            .find_export_of_class(FPackageObjectIndex::create_script_import(AK_AUDIO_EVENT_CLASS))
        else {
            continue;
        };
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
                location: CeMediaLocation::Loose(m.path.clone()),
                source_name: m.source_name.clone(),
            });
        }
        // Media-less event: the audio is inside the banks it lists, reachable
        // only through their event graph.
        if cooked.media.is_empty()
            && let Some((paks_root, store)) = banks.as_mut()
        {
            binding.media.extend(bank_embedded_media(paks_root, store, &cooked));
        }
        binding.events.push(cooked);
    }
    binding
}

/// Resolve an event whose media lives inside its SoundBanks: read each bank,
/// ask its event graph what `Play_…` triggers, and turn every source id into a
/// permutation. Streamed sources still resolve to a loose `.wem` when the pak
/// set actually stages one.
fn bank_embedded_media(
    paks_root: &Path,
    store: &mut CeMediaStore,
    event: &EventCookedData,
) -> Vec<CeSoundMedia> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<(String, u32)> = BTreeSet::new();
    for bank in &event.banks {
        let Ok(sources) = store.bank_event_sources(paks_root, &bank.path, event.event_id) else {
            continue;
        };
        for source in sources {
            if !seen.insert((bank.language.clone(), source.source_id)) {
                continue;
            }
            let loose = loose_media_path(source.source_id);
            let location = if source.streamed && store.has_media(paks_root, &loose) {
                CeMediaLocation::Loose(loose)
            } else {
                CeMediaLocation::Bank(bank.path.clone())
            };
            out.push(CeSoundMedia {
                event_name: event.event_name.clone(),
                language: bank.language.clone(),
                media_id: source.source_id,
                location,
                source_name: String::new(),
            });
        }
    }
    out
}

/// Where a streamed `.wem` is staged: `Media/<first two digits of id>/<id>.wem`,
/// the layout the cooked media paths use.
fn loose_media_path(media_id: u32) -> String {
    let id = media_id.to_string();
    let bucket = &id[..id.len().min(2)];
    format!("Media/{bucket}/{id}.wem")
}

/// The legacy `.pak` set holding Wwise media, opened lazily.
///
/// Opening every container parses each one's directory index, so this is done
/// once on first playback rather than at mount — a source can be browsed and
/// edited without ever touching audio.
#[derive(Default)]
pub struct CeMediaStore {
    /// The open set and the `Paks` directory it was opened from. The root is
    /// held because this store is shared by every workspace: without it, a
    /// second Campaign Evolved install reused the first one's containers and
    /// played its audio, or reported the media missing.
    paks: Option<(PathBuf, PakSet)>,
    /// Parsed event graph per bank path, keyed within the open pak set. Banks
    /// are shared by hundreds of tags (one per character, weapon, level), so
    /// re-parsing per tag would dominate the panel's frame cost.
    hirc: HashMap<String, Arc<HircIndex>>,
}

impl CeMediaStore {
    /// Open the pak set rooted at the source's `Paks` directory, if the set
    /// already open is not that one.
    fn paks(&mut self, paks_root: &Path) -> Result<&mut PakSet> {
        if self.paks.as_ref().is_none_or(|(root, _)| root != paks_root) {
            let set = PakSet::open_dir(paks_root)
                .with_context(|| format!("opening pak set at {}", paks_root.display()))?;
            if set.is_empty() {
                return Err(anyhow!("no readable .pak containers in {}", paks_root.display()));
            }
            self.paks = Some((paks_root.to_path_buf(), set));
            self.hirc.clear(); // the cache is only valid for the open set
        }
        Ok(&mut self.paks.as_mut().expect("just populated").1)
    }

    /// Whether the pak set stages a file at this Wwise-mount-relative path.
    fn has_media(&mut self, paks_root: &Path, rel_path: &str) -> bool {
        let path = format!("{WWISE_MOUNT}/{rel_path}");
        self.paks(paks_root).is_ok_and(|set| set.contains(&path))
    }

    /// The media sources one event triggers, according to a bank's own event
    /// graph. Used for events whose cooked data names banks but no media.
    fn bank_event_sources(
        &mut self,
        paks_root: &Path,
        bank_path: &str,
        event_id: u32,
    ) -> Result<Vec<SoundSource>> {
        if let Some(index) = self.hirc.get(bank_path) {
            return Ok(index.resolve_event_id(event_id));
        }
        let bnk = self.read_bank(paks_root, bank_path)?;
        let mut index = HircIndex::new();
        if let Some(hirc) = bnk.hirc_bytes() {
            index.add_hirc(hirc, bnk.version);
        }
        index.finalize();
        let index = Arc::new(index);
        self.hirc.insert(bank_path.to_owned(), index.clone());
        Ok(index.resolve_event_id(event_id))
    }

    /// Read and parse one SoundBank out of the pak set.
    fn read_bank(&mut self, paks_root: &Path, bank_path: &str) -> Result<Bnk> {
        let path = format!("{WWISE_MOUNT}/{bank_path}");
        let bytes = self
            .paks(paks_root)?
            .read(&path)
            .with_context(|| format!("reading {path} from the pak set"))?;
        Bnk::parse(bytes).map_err(|e| anyhow!("parsing {path}: {e}"))
    }

    /// Fetch and decode one media entry to PCM.
    pub fn decode(
        &mut self,
        paks_root: &Path,
        media: &CeSoundMedia,
    ) -> Result<blam_tags::audio::DecodedPcm> {
        let bytes = match &media.location {
            CeMediaLocation::Loose(_) => {
                let path = media.mounted_path();
                self.paks(paks_root)?
                    .read(&path)
                    .with_context(|| format!("reading {path} from the pak set"))?
            }
            CeMediaLocation::Bank(bank_path) => {
                let bnk = self.read_bank(paks_root, bank_path)?;
                bnk.embedded_wem(media.media_id)
                    .ok_or_else(|| {
                        anyhow!("{} holds no media {}", bank_path, media.media_id)
                    })?
                    .to_vec()
            }
        };
        blam_tags::audio::wwise::decode_wem(&bytes)
            .map_err(|e| anyhow!("decoding {}: {e}", media.location_label()))
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
            location: CeMediaLocation::Loose("Media/43/43030714.wem".into()),
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
            location: CeMediaLocation::Loose("Media/1/1.wem".into()),
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
            location: CeMediaLocation::Loose("Media/1/1.wem".into()),
            source_name: String::new(),
        };
        let binding = CeSoundBinding { events: Vec::new(), media: vec![sfx] };
        // A non-localized event has no English(US) entry; asking for one must
        // still play rather than silently returning nothing.
        assert_eq!(binding.media_for_language("English(US)").len(), 1);
        assert_eq!(binding.languages(), vec!["SFX".to_string()]);
    }

    fn media(language: &str, id: u32) -> CeSoundMedia {
        CeSoundMedia {
            event_name: "Play_X".into(),
            language: language.into(),
            media_id: id,
            location: CeMediaLocation::Loose(format!("Media/{language}/{id}.wem")),
            source_name: String::new(),
        }
    }

    /// Localized voice carries no `SFX` entry at all. Showing `SFX` by default
    /// rendered an empty player for every line of dialogue in the game.
    #[test]
    fn localized_only_binding_never_shows_an_empty_player() {
        let binding = CeSoundBinding {
            events: Vec::new(),
            media: vec![
                media("Chinese(PRC)", 1),
                media("English(US)", 2),
                media("German", 3),
            ],
        };

        // Nothing selected: prefer English rather than the alphabetical first.
        let shown = binding.language_to_show(None);
        assert_eq!(shown, "English(US)");
        assert_eq!(binding.media_for_language(&shown).len(), 1);

        // A selection this tag does carry is honoured.
        assert_eq!(binding.language_to_show(Some("German")), "German");

        // A selection it does not carry must still show something.
        let shown = binding.language_to_show(Some("Korean"));
        assert!(binding.languages().contains(&shown), "fell back to an absent language");
        assert!(!binding.media_for_language(&shown).is_empty());
    }

    /// The mirror case: a non-localized tag while the shared selector holds a
    /// language, which must not blank it either.
    #[test]
    fn sfx_only_binding_ignores_a_localized_selection() {
        let binding = CeSoundBinding { events: Vec::new(), media: vec![media("SFX", 1)] };
        assert_eq!(binding.language_to_show(Some("German")), "SFX");
        assert_eq!(binding.media_for_language("German").len(), 1);
    }

    /// Every root the audio graph reaches through has to stay walkable. Events
    /// themselves are found by class, but the walk still needs to *get* to them.
    #[test]
    fn every_audio_root_is_walkable() {
        assert!(is_audio_package("/game/audio/characters/elite/shield_pop_elite"));
        assert!(is_audio_package("/game/audio/audio_fi/character/elites/shield/play_x"));
        assert!(is_audio_package("/game/wwise/events/play_foo"));
        assert!(is_audio_package("/game/wwiseaudio/events/systemic/vo/play_bar"));
        assert!(!is_audio_package("/game/tags/sound/x-sound"));
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

        // One tag per shape the resolution has to handle. Each was silent at
        // some point because the walk assumed the previous one's shape.
        let cases = [
            "/Game/Tags/sound/005_sandbox/006_character/006_character_movement/\
             006_chm_ge_weaanim/006_chm_ge_weaanim_ar_reloadmaghit-sound",
            "/Game/Tags/sound/dialog/combat/bisenti/default/01_contact/ambush-sound",
            // Scripted mission dialogue — the shape that rendered an empty
            // player because it carries no SFX entry.
            "/Game/Tags/sound/scripted/vo_scr_m02halo/m02_00040_cortana-sound",
            // Its event sits under `/Game/Audio/Audio_FI/…` rather than either
            // `Events` root, so a path-prefix test never finds it.
            "/Game/Tags/sound/characters/elite/shield_pop_elite-sound",
            // Media cooked inside a SoundBank: the event names banks and no
            // media, so the permutations come from the bank's own event graph.
            "/Game/Tags/sound/characters/bodyfalls/brute_bodyfalls/brute_bodyfall_dirt-sound",
        ];

        for tag in cases {
            let tag = tag.replace(char::is_whitespace, "");
            let binding = resolve_sound_binding(
                &containers,
                &packages,
                &usmap,
                &tag,
                Some((root.as_path(), &mut store)),
            );
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

            // What the player would actually show with nothing selected must
            // never be empty for a tag that resolved media.
            let shown = binding.language_to_show(None);
            assert!(
                !binding.media_for_language(&shown).is_empty(),
                "{tag} shows no rows for default language {shown}"
            );
            println!("  default language shown: {shown}");

            // And the media has to actually decode to non-silent audio.
            for m in binding.media_for_language(&shown) {
                let pcm = store.decode(&root, m).expect("decode media");
                assert!(!pcm.samples.is_empty(), "{} decoded to nothing", m.location_label());
                let peak = pcm.samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
                assert!(peak > 0, "{} decoded to silence", m.location_label());
                println!(
                    "  {} [{}] {} ch {} Hz peak {peak}  ({})",
                    m.display_name(),
                    m.language,
                    pcm.channels,
                    pcm.sample_rate,
                    m.location_label()
                );
            }
        }
    }

    /// Coverage over every mounted sound tag. Most of the game's audio was
    /// invisible to three separate assumptions in turn (one event root, one
    /// package layout, one bank version), each of which failed silently — so
    /// the guard is a floor on how much of the game resolves, not a spot check.
    ///
    /// Run with:
    ///   CE_PAKS=/path/to/Meteorite/Content/Paks cargo test ce_audio -- --ignored --nocapture
    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn most_sound_tags_resolve_to_media() {
        use blam_tags::iostore::IoStoreArchive;
        use std::path::PathBuf;
        use std::sync::Arc;

        let root =
            PathBuf::from(std::env::var("CE_PAKS").expect("set CE_PAKS to the game's Content/Paks"));
        let mut utocs: Vec<PathBuf> = std::fs::read_dir(&root)
            .expect("read paks dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("utoc")))
            .filter(|p| !p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("global.utoc")))
            .collect();
        utocs.sort();

        let mut containers = Vec::new();
        let mut packages = ContainerPackageIndex::default();
        let mut tag_packages: Vec<String> = Vec::new();
        for utoc in utocs {
            let Ok(archive) = IoStoreArchive::open(&utoc) else { continue };
            let idx = containers.len();
            for e in archive.entries() {
                if let Some(pkg) = super::super::container_package_name(&e.path) {
                    packages.insert(pkg, idx, e.path.clone());
                }
            }
            for e in archive.ublock_entries() {
                if e.path.to_ascii_lowercase().ends_with("-sound.ubulk")
                    && let Some(pkg) = tag_package_for_rel_path(&e.path)
                {
                    tag_packages.push(pkg);
                }
            }
            containers.push(MountedContainer {
                utoc_path: utoc.clone(),
                chunk_label: utoc.file_stem().unwrap().to_string_lossy().into_owned(),
                archive: Arc::new(archive),
            });
        }
        tag_packages.sort();
        tag_packages.dedup();

        let usmap = Usmap::meteorite().expect("bundled usmap");
        let mut store = CeMediaStore::default();
        let mut bound = 0usize;
        let mut from_banks = 0usize;
        for pkg in &tag_packages {
            let binding =
                resolve_sound_binding(&containers, &packages, &usmap, pkg, Some((root.as_path(), &mut store)));
            if binding.is_empty() {
                continue;
            }
            bound += 1;
            if binding.media.iter().any(|m| matches!(m.location, CeMediaLocation::Bank(_))) {
                from_banks += 1;
            }
        }
        let total = tag_packages.len();
        println!("{bound}/{total} sound tags resolved media ({from_banks} out of SoundBanks)");
        // Measured 5332/5895 on the 2026.06.26 build; the rest are stubs with
        // no audio asset behind them at all.
        assert!(bound * 100 / total >= 88, "only {bound}/{total} sound tags resolved");
        assert!(from_banks > 400, "bank-embedded media stopped resolving ({from_banks})");
    }
}
