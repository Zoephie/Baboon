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
        if stem.is_empty() {
            self.media_id.to_string()
        } else {
            stem.to_string()
        }
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
        let exact: Vec<&CeSoundMedia> = self
            .media
            .iter()
            .filter(|m| m.language.eq_ignore_ascii_case(language))
            .collect();
        if !exact.is_empty() {
            return exact;
        }
        let sfx: Vec<&CeSoundMedia> = self
            .media
            .iter()
            .filter(|m| m.language.eq_ignore_ascii_case("SFX"))
            .collect();
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
        let has = |name: &str| {
            languages
                .iter()
                .find(|l| l.eq_ignore_ascii_case(name))
                .cloned()
        };

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
    let base = rel_path
        .strip_suffix(".ubulk")
        .or_else(|| rel_path.strip_suffix(".uasset"))?;
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
    let header = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        TOC_VERSION,
        HEADER_VERSION,
        None,
    )
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
    header.exports_class(FPackageObjectIndex::create_script_import(
        AK_AUDIO_EVENT_CLASS,
    ))
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
        let Some((header, bytes)) = read_package(containers, packages, &package) else {
            continue;
        };
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
        let Some(export) = header.find_export_of_class(FPackageObjectIndex::create_script_import(
            AK_AUDIO_EVENT_CLASS,
        )) else {
            continue;
        };
        let start = header.summary.header_size as usize + export.cooked_serial_offset as usize;
        let end = start + export.cooked_serial_size as usize;
        let Some(body) = bytes.get(start..end) else {
            continue;
        };
        let names = header.name_map.copy_raw_names();
        let Ok(cooked) = read_event_cooked_data(body, &names, usmap) else {
            continue;
        };

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
            binding
                .media
                .extend(bank_embedded_media(paks_root, store, &cooked));
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
                return Err(anyhow!(
                    "no readable .pak containers in {}",
                    paks_root.display()
                ));
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

    /// Read one media entry's encoded bytes out of the pak set.
    pub fn fetch(&mut self, paks_root: &Path, media: &CeSoundMedia) -> Result<Vec<u8>> {
        Ok(match &media.location {
            CeMediaLocation::Loose(_) => {
                let path = media.mounted_path();
                self.paks(paks_root)?
                    .read(&path)
                    .with_context(|| format!("reading {path} from the pak set"))?
            }
            CeMediaLocation::Bank(bank_path) => {
                let bnk = self.read_bank(paks_root, bank_path)?;
                bnk.embedded_wem(media.media_id)
                    .ok_or_else(|| anyhow!("{} holds no media {}", bank_path, media.media_id))?
                    .to_vec()
            }
        })
    }
}

/// Decode one media entry's bytes, as [`CeMediaStore::fetch`] returned them.
pub fn decode_media_bytes(
    media: &CeSoundMedia,
    bytes: &[u8],
) -> Result<blam_tags::audio::DecodedPcm> {
    blam_tags::audio::wwise::decode_wem(bytes)
        .map_err(|e| anyhow!("decoding {}: {e}", media.location_label()))
}

#[cfg(test)]
mod tests;
