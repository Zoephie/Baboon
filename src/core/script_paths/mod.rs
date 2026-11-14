//! Tag paths written as text in mission scripts.
//!
//! A scenario's scripts name tags by path in their source text:
//! `(custom_animation unit "objects\characters\hunter\hunter" …)`. The editing
//! kit recompiles that text whenever it loads the scenario, and the first path
//! that names no tag aborts the whole compile, so the mission loads with no
//! scripts at all. Moving, renaming or deleting a tag must therefore deal with
//! the text as well as with tag references.
//!
//! The text lives in up to three places, all handled here:
//! - `.hsc` files under the kit's `data` folder (what the tools compile);
//! - each scenario's `source files` block (a copy saved in the tag);
//! - `scenario_hs_source_file` tags.
//!
//! Script paths carry no extension: which tag one means depends on what the
//! call it is passed to expects, which [`types`] reads from the bundled
//! script docs. A path is rewritten only when it can mean just one tag (or
//! every tag it could mean moves to the same new path).
//!
//! The compiled script data (`script string data`, syntax datums) is left
//! alone: the tools rebuild it from the text on load.

mod hs;
mod types;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use blam_tags::{TagFieldData, TagFile};

use crate::core::folder_merge::Transaction;
use crate::core::game::GameId;
use crate::core::source::{KitLayout, TagEntry, TagEntryLocation, TagSource, read_entry};
use crate::core::tag_path;

use hs::Place;
use types::{Expected, Signatures, TagKind};

/// Whether Baboon checks a game's scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScriptSupport {
    /// Scripts are checked and fixed.
    Checked,
    /// The game keeps its script text in script tags Baboon does not check
    /// yet (Halo 2 Anniversary Multiplayer, Halo 4).
    NotChecked,
}

/// How Baboon handles `game`'s scripts; `None` when it has none to handle.
pub(crate) fn script_support(game: Option<GameId>) -> Option<ScriptSupport> {
    match game? {
        GameId::HaloCe | GameId::Halo2 | GameId::Halo3 | GameId::Halo3Odst | GameId::HaloReach => {
            Some(ScriptSupport::Checked)
        }
        GameId::Halo2Amp | GameId::Halo4 => Some(ScriptSupport::NotChecked),
        GameId::CampaignEvolved => None,
    }
}

/// The note for games whose scripts Baboon does not check.
pub(crate) const NOT_CHECKED_NOTE: &str =
    "Baboon doesn't check this game's script tags yet. Scripts that use these tags may need their paths fixed by hand.";

/// Whether tags are being renamed (moved) or deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    #[default]
    Rename,
    Delete,
}

/// The tags an operation changes, by path.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScriptChanges {
    kind: ChangeKind,
    /// Lowercased path without extension → each changed tag there: its
    /// group (extension) and its new path without extension (`None` when
    /// deleted, or when the new name is not known yet).
    by_base: HashMap<String, Vec<(String, Option<String>)>>,
}

impl ScriptChanges {
    pub fn new(kind: ChangeKind) -> Self {
        Self { kind, by_base: HashMap::new() }
    }

    /// Renames, as `(old, new)` tag paths with extensions. A `None` new path
    /// is a rename whose new name is not known yet: it is listed, but not
    /// rewritten.
    pub fn renaming<'a>(pairs: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> Self {
        let mut changes = Self::new(ChangeKind::Rename);
        for (old, new) in pairs {
            changes.add(old, new);
        }
        changes
    }

    /// Deletions, as tag paths with extensions.
    pub fn deleting<'a>(paths: impl IntoIterator<Item = &'a str>) -> Self {
        let mut changes = Self::new(ChangeKind::Delete);
        for path in paths {
            changes.add(path, None);
        }
        changes
    }

    fn add(&mut self, old: &str, new: Option<&str>) {
        let Some((base, group)) = tag_path::split_extension(old) else { return };
        let new = new.map(|new| tag_path::without_extension(new).to_owned());
        if new.as_deref().is_some_and(|new| new.eq_ignore_ascii_case(base)) {
            return;
        }
        self.by_base
            .entry(base.to_ascii_lowercase())
            .or_default()
            .push((group.to_ascii_lowercase(), new));
    }

    pub fn kind(&self) -> ChangeKind {
        self.kind
    }

    pub fn is_empty(&self) -> bool {
        self.by_base.is_empty()
    }
}

/// What happens to one path in a script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It is updated to the new path.
    Fixed,
    /// It could mean a moved tag or one that stays, so it is left alone.
    CantFix,
    /// Every tag it could mean is deleted.
    Missing,
    /// Some tag it could mean is deleted, and which one the script wants is
    /// unknown.
    MayBreak,
}

/// One path in a script that the operation affects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptHit {
    pub line: usize,
    /// The path as written.
    pub old: String,
    /// What it becomes, when known.
    pub new: Option<String>,
    pub outcome: Outcome,
    /// The type the script passes it as, as the script docs name it.
    pub expected: Option<String>,
}

/// One script the operation affects, across every copy of its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptImpact {
    /// `03a_oldmombasa_mission.hsc`.
    pub name: String,
    /// Where it sits, relative to the data or tags folder:
    /// `scenarios\solo\03a_oldmombasa\scripts`.
    pub folder: String,
    /// The affected paths in its first copy (the data folder's, if any),
    /// in order. Comments and plain text are left out.
    pub hits: Vec<ScriptHit>,
    /// Whether the `.hsc` file in the data folder exists.
    pub data_file: bool,
    /// How many copies saved in tags hold it.
    pub tag_copies: usize,
}

impl ScriptImpact {
    /// `scenarios\solo\03a_oldmombasa\scripts\03a_oldmombasa_mission.hsc`.
    pub fn path(&self) -> String {
        tag_path::join(&self.folder, &self.name)
    }

    pub fn count(&self, outcome: Outcome) -> usize {
        self.hits.iter().filter(|hit| hit.outcome == outcome).count()
    }
}

/// Every script an operation affects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScriptReport {
    pub kind: ChangeKind,
    pub scripts: Vec<ScriptImpact>,
}

/// What fixing scripts changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScriptFixSummary {
    /// Scripts with at least one path updated.
    pub scripts: usize,
    /// Paths updated (each script counted once, not per copy).
    pub fixed: usize,
    /// Paths left alone because they could mean more than one tag.
    pub cant_fix: usize,
    /// `.hsc` files and tags written.
    pub files_written: usize,
}

impl ScriptFixSummary {
    /// A line for the terminal, or `None` when no script was affected.
    pub fn line(&self) -> Option<String> {
        if self.fixed == 0 && self.cant_fix == 0 {
            return None;
        }
        let mut line = format!("Updated {} path(s) in {} script(s)", self.fixed, self.scripts);
        if self.cant_fix > 0 {
            line.push_str(&format!(
                "; {} script path(s) could mean more than one tag and were left unchanged",
                self.cant_fix
            ));
        }
        Some(line)
    }
}

/// The kit whose scripts an operation reaches.
pub(crate) struct ScriptKit<'a> {
    pub tags_root: &'a Path,
    pub game: GameId,
    /// For reading scenario tags.
    pub source: &'a TagSource,
    /// Every tag in the kit as it is before the operation.
    pub entries: &'a [TagEntry],
}

/// Which scripts `changes` affects, and how. Reads the data folder's `.hsc`
/// files and the kit's scenario and script source tags; changes nothing.
pub(crate) fn check_scripts(
    kit: &ScriptKit,
    changes: &ScriptChanges,
    cancelled: &dyn Fn() -> bool,
) -> Result<ScriptReport, String> {
    if changes.is_empty() || script_support(Some(kit.game)) != Some(ScriptSupport::Checked) {
        return Ok(ScriptReport { kind: changes.kind, scripts: Vec::new() });
    }
    let copies = collect(kit, changes, &|path| path.to_path_buf(), cancelled)?;
    Ok(ScriptReport { kind: changes.kind, scripts: impacts(&copies) })
}

/// Rewrite the paths `changes` renames in every copy of every script,
/// recording each file in `transaction` before it is written so a later
/// failure restores it. `relocate` maps a tag file's path before the
/// operation to where it is now (a scenario may have moved with its folder).
/// Deletions are never rewritten.
pub(crate) fn fix_scripts(
    kit: &ScriptKit,
    changes: &ScriptChanges,
    relocate: &dyn Fn(&Path) -> PathBuf,
    transaction: &mut Transaction,
) -> Result<ScriptFixSummary, String> {
    if changes.kind == ChangeKind::Delete
        || changes.is_empty()
        || script_support(Some(kit.game)) != Some(ScriptSupport::Checked)
    {
        return Ok(ScriptFixSummary::default());
    }
    let copies = collect(kit, changes, relocate, &|| false)?;
    let mut summary = ScriptFixSummary::default();
    for impact in impacts(&copies) {
        let fixed = impact.count(Outcome::Fixed);
        if fixed > 0 {
            summary.scripts += 1;
        }
        summary.fixed += fixed;
        summary.cant_fix += impact.count(Outcome::CantFix);
    }
    // One write per file: a tag can hold several copies, each its element
    // (see [`Location::Tag`]) and new text.
    type TagTexts = (TagEntry, Vec<(Option<usize>, Vec<u8>)>);
    let mut data_files = Vec::new();
    let mut tags: BTreeMap<PathBuf, TagTexts> = BTreeMap::new();
    for copy in copies {
        if copy.edits.is_empty() {
            continue;
        }
        let text = apply_edits(&copy.text, &copy.edits);
        match copy.location {
            Location::DataFile(path) => data_files.push((path, text)),
            Location::Tag { entry, path, element } => {
                tags.entry(path).or_insert_with(|| (entry, Vec::new())).1.push((element, text));
            }
        }
    }
    for (path, text) in data_files {
        transaction.will_modify(&path)?;
        write_file(&path, &text).map_err(|error| format!("Could not update script {}: {error}", path.display()))?;
        summary.files_written += 1;
    }
    for (path, (entry, texts)) in tags {
        let mut tag = read_entry(kit.source, &entry)
            .map_err(|error| format!("Could not read {} to update its scripts: {error}", entry.display_path))?;
        for (element, text) in texts {
            let field = match element {
                Some(index) => format!("source files[{index}]/source"),
                None => "source".to_owned(),
            };
            tag.root_mut()
                .field_path_mut(&field)
                .ok_or_else(|| format!("{} has no {field} field", entry.display_path))?
                .set(TagFieldData::Data(text))
                .map_err(|error| format!("Could not update a script in {}: {error:?}", entry.display_path))?;
        }
        transaction.will_modify(&path)?;
        tag.write_atomic(&path)
            .map_err(|error| format!("Could not update the scripts in {}: {error}", entry.display_path))?;
        summary.files_written += 1;
    }
    Ok(summary)
}

/// Where one copy of a script's text is.
#[derive(Clone)]
enum Location {
    DataFile(PathBuf),
    /// In a tag: element `element` of its `source files` block, or (`None`)
    /// the tag's own `source` field.
    Tag { entry: TagEntry, path: PathBuf, element: Option<usize> },
}

/// One copy of a script, scanned.
struct ScannedCopy {
    name: String,
    folder: String,
    location: Location,
    text: Vec<u8>,
    hits: Vec<ScriptHit>,
    /// Byte ranges to replace, in order.
    edits: Vec<(usize, usize, String)>,
}

fn collect(
    kit: &ScriptKit,
    changes: &ScriptChanges,
    relocate: &dyn Fn(&Path) -> PathBuf,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<ScannedCopy>, String> {
    let cancel = || -> Result<(), String> {
        if cancelled() { Err("Cancelled".to_owned()) } else { Ok(()) }
    };
    let resolver = Resolver::new(kit, changes);
    let needles = resolver.needles();
    let mut copies = Vec::new();
    let mut scan = |name: String, folder: String, location: Location, text: Vec<u8>| {
        let (hits, edits) = resolver.scan(&text);
        if !hits.is_empty() || !edits.is_empty() {
            copies.push(ScannedCopy { name, folder, location, text, hits, edits });
        }
    };

    if let Some(layout) = KitLayout::from_tags_folder(kit.tags_root) {
        for path in hsc_files(&layout.data) {
            cancel()?;
            let Ok(text) = fs::read(&path) else { continue };
            if !contains_any(&text, &needles) {
                continue;
            }
            let relative = path.strip_prefix(&layout.data).unwrap_or(&path);
            let folder = relative.parent().map(tag_path::relative_tag_path).unwrap_or_default();
            let name = relative.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned();
            scan(name, folder, Location::DataFile(path), text);
        }
    }

    for entry in kit.entries {
        let TagEntryLocation::LooseFile(original) = &entry.location else { continue };
        let Some((stem, group)) = tag_path::split_extension(&entry.display_path) else { continue };
        let scenario = group.eq_ignore_ascii_case("scenario");
        if !scenario && !group.eq_ignore_ascii_case("scenario_hs_source_file") {
            continue;
        }
        cancel()?;
        let path = relocate(original);
        let Ok(bytes) = fs::read(&path) else { continue };
        if !contains_any(&bytes, &needles) {
            continue;
        }
        let mut current = entry.clone();
        current.location = TagEntryLocation::LooseFile(path.clone());
        let Ok(tag) = read_entry(kit.source, &current) else { continue };
        let parent = tag_path::parent(&entry.display_path).unwrap_or("");
        if scenario {
            let folder = tag_path::join(parent, "scripts");
            for (index, name, text) in scenario_sources(&tag) {
                let location = Location::Tag { entry: current.clone(), path: path.clone(), element: Some(index) };
                scan(format!("{name}.hsc"), folder.clone(), location, text);
            }
        } else if let Some(TagFieldData::Data(text)) = tag.root().field_path("source").and_then(|field| field.value()) {
            let name = format!("{}.hsc", tag_path::name(stem));
            let location = Location::Tag { entry: current.clone(), path: path.clone(), element: None };
            scan(name, parent.to_owned(), location, text);
        }
    }
    Ok(copies)
}

/// A scenario's saved script copies: element index, name and text.
fn scenario_sources(tag: &TagFile) -> Vec<(usize, String, Vec<u8>)> {
    let root = tag.root();
    let Some(block) = root.field_path("source files").and_then(|field| field.as_block()) else {
        return Vec::new();
    };
    (0..block.len())
        .filter_map(|index| {
            let element = block.element(index)?;
            let name = match element.field_path("name")?.value()? {
                TagFieldData::String(name) | TagFieldData::LongString(name) => name,
                _ => return None,
            };
            match element.field_path("source")?.value()? {
                TagFieldData::Data(text) => Some((index, name, text)),
                _ => None,
            }
        })
        .collect()
}

/// Group copies into one impact per script, keyed by folder and name; the
/// data folder's copy speaks for the script when there is one.
fn impacts(copies: &[ScannedCopy]) -> Vec<ScriptImpact> {
    let mut by_key: BTreeMap<(String, String), Vec<&ScannedCopy>> = BTreeMap::new();
    for copy in copies {
        by_key
            .entry((copy.folder.to_ascii_lowercase(), copy.name.to_ascii_lowercase()))
            .or_default()
            .push(copy);
    }
    let mut impacts: Vec<_> = by_key
        .into_values()
        .filter_map(|group| {
            let from_data = |copy: &&&ScannedCopy| matches!(copy.location, Location::DataFile(_));
            let speaker = group
                .iter()
                .find(|copy| from_data(copy) && !copy.hits.is_empty())
                .or_else(|| group.iter().find(|copy| !copy.hits.is_empty()))?;
            Some(ScriptImpact {
                name: speaker.name.clone(),
                folder: speaker.folder.clone(),
                hits: speaker.hits.clone(),
                data_file: group.iter().any(|copy| from_data(&copy)),
                tag_copies: group.iter().filter(|copy| !from_data(copy)).count(),
            })
        })
        .collect();
    impacts.sort_by_cached_key(|impact| crate::core::source::natural_key(&impact.path()));
    impacts
}

/// Resolves the paths in script text against the kit and the changes.
struct Resolver<'a> {
    changes: &'a ScriptChanges,
    /// Lowercased path without extension → the groups of the tags there,
    /// before the operation. Only the paths the changes touch.
    catalog: HashMap<String, Vec<String>>,
    signatures: std::sync::Arc<Signatures>,
    objects: std::sync::Arc<std::collections::HashSet<String>>,
}

/// What one path resolves to.
struct Resolution {
    outcome: Outcome,
    /// The new path, as it should be written, when known.
    new: Option<String>,
}

impl<'a> Resolver<'a> {
    fn new(kit: &ScriptKit, changes: &'a ScriptChanges) -> Self {
        let mut catalog: HashMap<String, Vec<String>> = HashMap::new();
        for entry in kit.entries {
            let Some((base, group)) = tag_path::split_extension(&entry.display_path) else { continue };
            let base = base.to_ascii_lowercase();
            if changes.by_base.contains_key(&base) {
                catalog.entry(base).or_default().push(group.to_ascii_lowercase());
            }
        }
        // A change for a tag the entries do not list still counts.
        for (base, groups) in &changes.by_base {
            let known = catalog.entry(base.clone()).or_default();
            for (group, _) in groups {
                if !known.contains(group) {
                    known.push(group.clone());
                }
            }
        }
        Self { changes, catalog, signatures: Signatures::for_game(kit.game), objects: types::object_groups(kit.game) }
    }

    /// Lowercased byte strings any affected script must contain: the folders
    /// the changed tags sit in, each once, and only the outermost of nested
    /// ones. A whole folder's tags are one search (`objects\characters\
    /// heretic\`), not one per tag; a script that mentions the folder is then
    /// read in full.
    fn needles(&self) -> Vec<Vec<u8>> {
        needles_for(self.changes.by_base.keys().map(String::as_str))
    }

    fn accepts(&self, kind: &TagKind, group: &str) -> bool {
        match kind {
            TagKind::Groups(groups) => groups.contains(&group),
            TagKind::Objects => self.objects.contains(group),
            TagKind::Shaders => group.starts_with("shader"),
            TagKind::Any => true,
        }
    }

    fn resolve(&self, written: &str, expected: &Expected) -> Option<Resolution> {
        let normalized = written.replace('/', "\\").to_ascii_lowercase();
        let (base, explicit) = match tag_path::split_extension(&normalized) {
            Some((base, extension))
                if self.catalog.get(base).is_some_and(|groups| groups.iter().any(|group| group == extension)) =>
            {
                (base.to_owned(), Some(extension.to_owned()))
            }
            _ => (normalized.clone(), None),
        };
        let groups = self.catalog.get(&base)?;
        let changed = self.changes.by_base.get(&base)?;
        let (candidates, typed): (Vec<&String>, bool) = match (&explicit, expected) {
            (Some(extension), _) => (groups.iter().filter(|group| *group == extension).collect(), true),
            (None, Expected::Tag { kind, .. }) => {
                let fitting: Vec<_> = groups.iter().filter(|group| self.accepts(kind, group)).collect();
                if fitting.is_empty() {
                    (groups.iter().collect(), false)
                } else {
                    (fitting, *kind != TagKind::Any)
                }
            }
            (None, _) => (groups.iter().collect(), false),
        };
        let mut news = Vec::new();
        let mut unchanged = 0;
        for group in candidates {
            match changed.iter().find(|(changed, _)| changed == group) {
                Some((_, new)) => news.push(new.clone()),
                None => unchanged += 1,
            }
        }
        if news.is_empty() {
            return None;
        }
        let resolution = match self.changes.kind {
            ChangeKind::Rename => {
                let first = news[0].clone();
                if unchanged == 0 && news.iter().all(|new| new == &first) {
                    let new = first.map(|new| written_like(written, &new, explicit.as_deref()));
                    Resolution { outcome: Outcome::Fixed, new }
                } else {
                    Resolution { outcome: Outcome::CantFix, new: None }
                }
            }
            ChangeKind::Delete if unchanged == 0 => Resolution { outcome: Outcome::Missing, new: None },
            // Another tag of the right type remains for it to mean.
            ChangeKind::Delete if typed => return None,
            ChangeKind::Delete => Resolution { outcome: Outcome::MayBreak, new: None },
        };
        Some(resolution)
    }

    /// The hits to report and the edits to make in one copy of a script.
    fn scan(&self, text: &[u8]) -> (Vec<ScriptHit>, Vec<(usize, usize, String)>) {
        let mut hits = Vec::new();
        let mut edits = Vec::new();
        for token in hs::path_tokens(text) {
            let Ok(written) = std::str::from_utf8(token.text(text)) else { continue };
            let expected = match &token.place {
                Place::Argument { function, index } => self.signatures.expected(function, *index),
                Place::GlobalValue { value_type } => types::classify(value_type),
                Place::Unknown => Expected::Unknown,
                Place::Text => Expected::NotTag,
            };
            if expected == Expected::NotTag {
                // Not compiled as a tag: update it when that is unambiguous,
                // but it never breaks a script, so it is not reported.
                if let Some(Resolution { outcome: Outcome::Fixed, new: Some(new) }) =
                    self.resolve(written, &Expected::Unknown)
                {
                    edits.push((token.start, token.end, new));
                }
                continue;
            }
            let Some(resolution) = self.resolve(written, &expected) else { continue };
            if let (Outcome::Fixed, Some(new)) = (resolution.outcome, &resolution.new) {
                edits.push((token.start, token.end, new.clone()));
            }
            hits.push(ScriptHit {
                line: token.line,
                old: written.to_owned(),
                new: resolution.new,
                outcome: resolution.outcome,
                expected: match expected {
                    Expected::Tag { name, .. } => Some(name),
                    _ => None,
                },
            });
        }
        (hits, edits)
    }
}

/// `new` (a tag path without extension) written in the style of `written`:
/// its extension if it had one, its separators, and lowercase if it was.
fn written_like(written: &str, new: &str, extension: Option<&str>) -> String {
    let mut text = match extension {
        Some(extension) => {
            // Keep the extension exactly as written.
            let suffix = &written[written.len() - extension.len()..];
            format!("{new}.{suffix}")
        }
        None => new.to_owned(),
    };
    if !written.bytes().any(|byte| byte.is_ascii_uppercase()) {
        text = text.to_ascii_lowercase();
    }
    if written.contains('/') && !written.contains('\\') {
        text = text.replace('\\', "/");
    }
    text
}

fn apply_edits(text: &[u8], edits: &[(usize, usize, String)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 64);
    let mut at = 0;
    for (start, end, new) in edits {
        out.extend_from_slice(&text[at..*start]);
        out.extend_from_slice(new.as_bytes());
        at = *end;
    }
    out.extend_from_slice(&text[at..]);
    out
}

/// The search strings for tags at `bases` (lowercased paths without
/// extensions): each tag's folder with its trailing separator, or the path
/// itself for a tag at the root. Folders inside another one already searched
/// for are dropped. Each also in `/` form, which scripts may use.
fn needles_for<'a>(bases: impl Iterator<Item = &'a str>) -> Vec<Vec<u8>> {
    let mut prefixes: Vec<String> = bases
        .map(|base| match base.rfind('\\') {
            Some(separator) => base[..=separator].to_owned(),
            None => base.to_owned(),
        })
        .collect();
    prefixes.sort();
    prefixes.dedup();
    // Sorted, a folder comes straight before the folders inside it.
    let mut kept: Vec<String> = Vec::new();
    for prefix in prefixes {
        if !kept.last().is_some_and(|outer| outer.ends_with('\\') && prefix.starts_with(outer.as_str())) {
            kept.push(prefix);
        }
    }
    let mut needles = Vec::new();
    for prefix in kept {
        if prefix.contains('\\') {
            needles.push(prefix.replace('\\', "/").into_bytes());
        }
        needles.push(prefix.into_bytes());
    }
    needles
}

fn contains_any(bytes: &[u8], needles: &[Vec<u8>]) -> bool {
    let lowered = bytes.to_ascii_lowercase();
    needles.iter().any(|needle| !needle.is_empty() && memchr::memmem::find(&lowered, needle).is_some())
}

/// Every `.hsc` file under `data`.
fn hsc_files(data: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut folders = vec![data.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(listing) = fs::read_dir(&folder) else { continue };
        for entry in listing.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                folders.push(path);
            } else if kind.is_file()
                && path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("hsc"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Write through a temporary file beside `path`, so a failed write leaves
/// the original whole.
fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("hsc.baboon-write");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests;
