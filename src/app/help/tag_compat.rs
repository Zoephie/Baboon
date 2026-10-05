//! Lazy, read-only access to the generated cross-game tag compatibility
//! database.
//! It owns querying and presenting what the schemas say; deriving it belongs to
//! `crate::tag_compat_build` and acting on it to `app::conversion`.

use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

const TAG_COMPAT_FILE: &str = "tag_compat.sqlite3";
const TAG_COMPAT_SCHEMA_VERSION: i64 = 1;

/// What happens to a field, struct or group crossing a profile pair. Mirrors
/// the generator's `CompatVerdict`; kept separate so the read side does not
/// depend on the build side, which pulls in `rusqlite` write paths and the
/// whole schema walker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::app) enum CompatVerdict {
    HardBlocked,
    /// A verdict this build does not know, from a database a newer generator
    /// wrote. Reading it as `Identical` would claim the field "transfers
    /// unchanged" when nothing is known about it, so it counts as a loss
    /// until this build learns what it means.
    Unknown,
    SourceOnly,
    OptionLoss,
    TypeChangedSafe,
    RenamedProvable,
    TargetOnly,
    Identical,
}

impl CompatVerdict {
    fn parse(value: &str) -> Self {
        match value {
            "hard_blocked" => Self::HardBlocked,
            "source_only" => Self::SourceOnly,
            "option_loss" => Self::OptionLoss,
            "type_changed_safe" => Self::TypeChangedSafe,
            "renamed_provable" => Self::RenamedProvable,
            "target_only" => Self::TargetOnly,
            "identical" => Self::Identical,
            _ => Self::Unknown,
        }
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::HardBlocked => "blocked",
            Self::Unknown => "unknown",
            Self::SourceOnly => "dropped",
            Self::OptionLoss => "options lost",
            Self::TypeChangedSafe => "re-encoded",
            Self::RenamedProvable => "renamed",
            Self::TargetOnly => "default",
            Self::Identical => "identical",
        }
    }

    pub(in crate::app) fn explain(self) -> &'static str {
        match self {
            Self::HardBlocked => "cannot be converted",
            Self::Unknown => "not known to this version of Baboon; treat it as lost",
            Self::SourceOnly => "dropped — the target has no such field",
            Self::OptionLoss => "some options have no counterpart",
            Self::TypeChangedSafe => "the same value, re-encoded",
            Self::RenamedProvable => "renamed, and the rename is recorded",
            Self::TargetOnly => "left at its default — the source has no such field",
            Self::Identical => "transfers unchanged",
        }
    }

    /// Whether this costs the author anything. Drives the default filter: a
    /// reader opening the window wants the losses, not the 30,000 rows that
    /// transfer fine.
    pub(in crate::app) fn is_loss(self) -> bool {
        matches!(
            self,
            Self::HardBlocked | Self::Unknown | Self::SourceOnly | Self::OptionLoss
        )
    }

    /// Every verdict the database spells, so callers can derive a set rather
    /// than restate one. `Unknown` is every other spelling, so it has none.
    const ALL: [Self; 7] = [
        Self::HardBlocked,
        Self::SourceOnly,
        Self::OptionLoss,
        Self::TypeChangedSafe,
        Self::RenamedProvable,
        Self::TargetOnly,
        Self::Identical,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::HardBlocked => "hard_blocked",
            Self::Unknown => "unknown",
            Self::SourceOnly => "source_only",
            Self::OptionLoss => "option_loss",
            Self::TypeChangedSafe => "type_changed_safe",
            Self::RenamedProvable => "renamed_provable",
            Self::TargetOnly => "target_only",
            Self::Identical => "identical",
        }
    }

    pub(in crate::app) fn color(self) -> Color32 {
        match self {
            Self::HardBlocked => material_delete_text(),
            Self::Unknown | Self::SourceOnly | Self::OptionLoss => Color32::from_rgb(242, 196, 48),
            Self::Identical => disclosure_triangle_green(),
            _ => subtle_dark(),
        }
    }
}

/// The `IN (...)` list of the verdicts that cost nothing, built from
/// [`CompatVerdict::is_loss`] rather than written out. The loss filter keeps
/// rows *not* in it, so a verdict this build cannot read stays in view as the
/// loss it is treated as.
///
/// Two queries filter on it, and a third would be easy to add. Spelling the set
/// into SQL by hand is how one of them ends up disagreeing with the checkbox
/// that claims to control it.
fn lossless_verdict_sql() -> String {
    CompatVerdict::ALL
        .iter()
        .filter(|verdict| !verdict.is_loss())
        .map(|verdict| format!("'{}'", verdict.as_str()))
        .collect::<Vec<_>>()
        .join(",")
}

/// An ordered profile pair the database covers.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct CompatPair {
    pub(in crate::app) id: i64,
    pub(in crate::app) source_game: String,
    pub(in crate::app) target_game: String,
}

impl CompatPair {
    pub(in crate::app) fn label(&self) -> String {
        format!("{} → {}", self.source_game, self.target_game)
    }
}

#[derive(Clone)]
pub(in crate::app) struct CompatGroupRow {
    pub(in crate::app) group: String,
    pub(in crate::app) verdict: CompatVerdict,
    pub(in crate::app) size_diff_structs: i64,
    pub(in crate::app) source_only_fields: i64,
    pub(in crate::app) target_only_fields: i64,
    pub(in crate::app) blocked_reason: Option<String>,
}

#[derive(Clone)]
pub(in crate::app) struct CompatFieldRow {
    pub(in crate::app) struct_key: String,
    pub(in crate::app) first_path: String,
    pub(in crate::app) source_name: Option<String>,
    pub(in crate::app) source_type: Option<String>,
    pub(in crate::app) target_name: Option<String>,
    pub(in crate::app) target_type: Option<String>,
    pub(in crate::app) verdict: CompatVerdict,
    pub(in crate::app) rule: String,
    pub(in crate::app) detail: String,
}

enum CompatDatabase {
    Unloaded,
    Loaded(Connection),
    Failed(String),
}

pub(in crate::app) struct TagCompatUiState {
    database: CompatDatabase,
    pub(in crate::app) pairs: Vec<CompatPair>,
    pub(in crate::app) pair: usize,
    pub(in crate::app) losses_only: bool,
    pub(in crate::app) search: String,
    pub(in crate::app) selected_group: Option<String>,
    pub(in crate::app) groups: Vec<CompatGroupRow>,
    pub(in crate::app) fields: Vec<CompatFieldRow>,
    last_query: Option<(usize, bool, String)>,
    last_group: Option<(usize, String, bool)>,
}

impl Default for TagCompatUiState {
    fn default() -> Self {
        Self {
            database: CompatDatabase::Unloaded,
            pairs: Vec::new(),
            pair: 0,
            // A reader opens this to find out what they will lose. Thirty
            // thousand rows that transfer fine are not the answer.
            losses_only: true,
            search: String::new(),
            selected_group: None,
            groups: Vec::new(),
            fields: Vec::new(),
            last_query: None,
            last_group: None,
        }
    }
}

impl TagCompatUiState {
    pub(in crate::app) fn error(&self) -> Option<&str> {
        match &self.database {
            CompatDatabase::Failed(error) => Some(error),
            _ => None,
        }
    }

    pub(in crate::app) fn ensure_loaded(&mut self, docs_root: &Path) {
        if !matches!(self.database, CompatDatabase::Unloaded) {
            return;
        }
        let path = docs_root.join(TAG_COMPAT_FILE);
        self.database = match open_database(&path) {
            Ok(connection) => match query_pairs(&connection) {
                Ok(pairs) => {
                    self.pairs = pairs;
                    CompatDatabase::Loaded(connection)
                }
                Err(error) => CompatDatabase::Failed(error),
            },
            Err(error) => CompatDatabase::Failed(error),
        };
    }

    /// Point the window at a specific pair and group — the hook the import and
    /// conversion dialogs use so "what transfers for this group?" lands on the
    /// answer rather than on a search box.
    pub(in crate::app) fn focus(&mut self, source_game: &str, target_game: &str, group: &str) {
        if let Some(index) = self
            .pairs
            .iter()
            .position(|pair| pair.source_game == source_game && pair.target_game == target_game)
        {
            self.pair = index;
        }
        // A focused group is one the caller already knows is interesting, so
        // show all of it rather than only its losses.
        self.losses_only = false;
        self.search = group.to_owned();
        self.selected_group = Some(group.to_owned());
        self.last_query = None;
        self.last_group = None;
    }

    pub(in crate::app) fn refresh(&mut self) {
        let query = (self.pair, self.losses_only, self.search.clone());
        if self.last_query.as_ref() != Some(&query) {
            let CompatDatabase::Loaded(connection) = &self.database else {
                return;
            };
            let Some(pair) = self.pairs.get(self.pair) else {
                return;
            };
            match query_groups(connection, pair.id, self.losses_only, &self.search) {
                Ok(groups) => {
                    self.groups = groups;
                    self.last_query = Some(query);
                    if self
                        .selected_group
                        .as_ref()
                        .is_some_and(|name| self.groups.iter().all(|row| &row.group != name))
                    {
                        self.selected_group = None;
                        self.fields.clear();
                    }
                }
                Err(error) => {
                    self.database = CompatDatabase::Failed(error);
                    return;
                }
            }
        }

        let Some(group) = self.selected_group.clone() else {
            self.fields.clear();
            return;
        };
        let key = (self.pair, group.clone(), self.losses_only);
        if self.last_group.as_ref() == Some(&key) {
            return;
        }
        let CompatDatabase::Loaded(connection) = &self.database else {
            return;
        };
        let Some(pair) = self.pairs.get(self.pair) else {
            return;
        };
        match query_fields(connection, pair.id, &group, self.losses_only) {
            Ok(fields) => {
                self.fields = fields;
                self.last_group = Some(key);
            }
            Err(error) => self.database = CompatDatabase::Failed(error),
        }
    }

    pub(in crate::app) fn select_group(&mut self, group: String) {
        if self.selected_group.as_ref() == Some(&group) {
            return;
        }
        self.selected_group = Some(group);
        self.last_group = None;
    }

    /// The currently visible rows as CSV — the "sheet" a reader takes away.
    /// Exports what is on screen, filters included, so what they get is what
    /// they were looking at.
    pub(in crate::app) fn visible_csv(&self) -> String {
        let pair = self.pairs.get(self.pair);
        let (source, target) = pair
            .map(|pair| (pair.source_game.as_str(), pair.target_game.as_str()))
            .unwrap_or(("", ""));
        let mut out = String::from(
            "source_game,target_game,group,struct,path,source_field,source_type,\
             target_field,target_type,verdict,rule,detail\n",
        );
        let group = self.selected_group.as_deref().unwrap_or("");
        for row in &self.fields {
            let cells = [
                source,
                target,
                group,
                &row.struct_key,
                &row.first_path,
                row.source_name.as_deref().unwrap_or(""),
                row.source_type.as_deref().unwrap_or(""),
                row.target_name.as_deref().unwrap_or(""),
                row.target_type.as_deref().unwrap_or(""),
                row.verdict.label(),
                &row.rule,
                &row.detail,
            ];
            out.push_str(&csv_row(&cells));
            out.push('\n');
        }
        out
    }
}

fn csv_row(cells: &[&str]) -> String {
    cells
        .iter()
        .map(|cell| {
            if cell.contains([',', '"', '\n']) {
                format!("\"{}\"", cell.replace('"', "\"\""))
            } else {
                (*cell).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn open_database(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not read the tag compatibility schema: {error}"))?;
    if version
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok())
        != Some(TAG_COMPAT_SCHEMA_VERSION)
    {
        return Err(format!(
            "Unsupported tag compatibility schema in {} (expected version {}). \
             Rebuild it with `cargo run --bin build_tag_compat`.",
            path.display(),
            TAG_COMPAT_SCHEMA_VERSION,
        ));
    }
    Ok(connection)
}

fn query_pairs(connection: &Connection) -> Result<Vec<CompatPair>, String> {
    let mut statement = connection
        .prepare("SELECT pair_id,source_game,target_game FROM pairs ORDER BY pair_id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(CompatPair {
                id: row.get(0)?,
                source_game: row.get(1)?,
                target_game: row.get(2)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn query_groups(
    connection: &Connection,
    pair: i64,
    losses_only: bool,
    search: &str,
) -> Result<Vec<CompatGroupRow>, String> {
    let sql = format!(
        "SELECT group_name,verdict,size_diff_structs,source_only_fields,target_only_fields,blocked_reason
         FROM groups
         WHERE pair_id=?1
           AND (?2=0 OR verdict NOT IN ({}))
           AND (?3='' OR group_name LIKE '%'||?3||'%')
         ORDER BY group_name",
        lossless_verdict_sql(),
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![pair, losses_only as i64, search.trim()], |row| {
            Ok(CompatGroupRow {
                group: row.get(0)?,
                verdict: CompatVerdict::parse(&row.get::<_, String>(1)?),
                size_diff_structs: row.get(2)?,
                source_only_fields: row.get(3)?,
                target_only_fields: row.get(4)?,
                blocked_reason: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn query_fields(
    connection: &Connection,
    pair: i64,
    group: &str,
    losses_only: bool,
) -> Result<Vec<CompatFieldRow>, String> {
    let sql = format!(
        "SELECT f.struct_key,COALESCE(s.first_path,f.struct_key),
                f.source_name,f.source_type,f.target_name,f.target_type,
                f.verdict,f.rule,f.detail
         FROM fields f
         LEFT JOIN structs s
           ON s.pair_id=f.pair_id AND s.group_name=f.group_name AND s.struct_key=f.struct_key
         WHERE f.pair_id=?1 AND f.group_name=?2
           AND (?3=0 OR f.verdict NOT IN ({}))
         ORDER BY s.first_path,f.ordinal",
        lossless_verdict_sql(),
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![pair, group, losses_only as i64], |row| {
            Ok(CompatFieldRow {
                struct_key: row.get(0)?,
                first_path: row.get(1)?,
                source_name: row.get(2)?,
                source_type: row.get(3)?,
                target_name: row.get(4)?,
                target_type: row.get(5)?,
                verdict: CompatVerdict::parse(&row.get::<_, String>(6)?),
                rule: row.get(7)?,
                detail: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    //! Reading the shipped compatibility database at runtime.
    //!
    //! The generator's own tests prove the data is right. These prove the app can
    //! open it, that the queries answer the questions the window asks, and that the
    //! two halves agree — a database the build step is happy with and the reader
    //! cannot open is worse than no database.

    use super::*;

    fn database() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs")
    }

    fn loaded() -> TagCompatUiState {
        let mut state = TagCompatUiState::default();
        state.ensure_loaded(&database());
        assert!(state.error().is_none(), "{:?}", state.error());
        state
    }

    #[test]
    fn the_shipped_database_opens_and_covers_both_directions() {
        let state = loaded();
        let labels: Vec<String> = state.pairs.iter().map(CompatPair::label).collect();
        assert_eq!(
            labels,
            vec![
                "haloreach_mcc → haloce_evolved".to_owned(),
                "haloce_evolved → haloreach_mcc".to_owned(),
            ],
            "the sheet has to answer both directions",
        );
    }

    #[test]
    fn a_missing_database_reports_rather_than_panics() {
        let mut state = TagCompatUiState::default();
        state.ensure_loaded(Path::new("no/such/directory"));
        assert!(
            state.error().is_some(),
            "a missing file must be reported, not ignored"
        );
    }

    /// The window opens filtered to losses, because thirty thousand rows that
    /// transfer fine are not what a reader came for.
    #[test]
    fn the_default_view_shows_only_what_is_lost() {
        let mut state = loaded();
        assert!(state.losses_only, "losses are the default view");
        state.refresh();
        assert!(!state.groups.is_empty(), "some groups do lose something");
        assert!(
            state.groups.iter().all(|row| row.verdict.is_loss()),
            "the filter must not leak clean groups into the loss view",
        );
    }

    /// Turning the filter off has to bring back the groups that convert cleanly,
    /// or "only what is lost" is not a filter, it is the only view.
    #[test]
    fn clearing_the_filter_shows_the_clean_groups_too() {
        let mut state = loaded();
        state.refresh();
        let lossy = state.groups.len();
        state.losses_only = false;
        state.refresh();
        assert!(
            state.groups.len() > lossy,
            "unfiltered must be a superset: {} vs {lossy}",
            state.groups.len(),
        );
        assert!(
            state.groups.iter().any(|row| row.group == "sound_looping"),
            "sound_looping converts cleanly and should appear once the filter is off",
        );
    }

    /// The animation graph is the group this whole effort exists for, so the window
    /// has to be able to answer for it specifically.
    #[test]
    fn the_animation_graph_reports_its_losses_with_locations() {
        let mut state = loaded();
        state.focus("haloreach_mcc", "haloce_evolved", "model_animation_graph");
        state.refresh();

        assert_eq!(
            state.selected_group.as_deref(),
            Some("model_animation_graph")
        );
        assert!(!state.fields.is_empty(), "there are rows to show");

        let dropped: Vec<&str> = state
            .fields
            .iter()
            .filter(|row| row.verdict == CompatVerdict::SourceOnly)
            .filter_map(|row| row.source_name.as_deref())
            .collect();
        for expected in ["node joint flags", "additional flags"] {
            assert!(
                dropped.contains(&expected),
                "{expected} is dropped: {dropped:?}"
            );
        }

        let renamed = state
            .fields
            .iter()
            .find(|row| row.verdict == CompatVerdict::RenamedProvable)
            .expect("the blend-screen weight source is a recorded rename");
        assert!(
            !renamed.first_path.is_empty(),
            "every row needs a location — 'a field changed' is not actionable on an 83-struct group",
        );
    }

    /// `focus` is the hook the import dialog uses. It has to land on the answer,
    /// including turning off the loss filter: a caller asking about a specific
    /// group wants all of it.
    #[test]
    fn focusing_a_group_selects_it_and_shows_everything() {
        let mut state = loaded();
        state.focus("haloreach_mcc", "haloce_evolved", "sound_looping");
        state.refresh();
        assert!(!state.losses_only);
        assert_eq!(state.selected_group.as_deref(), Some("sound_looping"));
        assert_eq!(state.pairs[state.pair].source_game, "haloreach_mcc");
    }

    /// The export is the "sheet" deliverable, and it exports what is on screen —
    /// so a reader gets what they were looking at, not a different query.
    #[test]
    fn the_export_matches_what_is_on_screen() {
        let mut state = loaded();
        state.focus("haloreach_mcc", "haloce_evolved", "model_animation_graph");
        state.refresh();
        let csv = state.visible_csv();

        let lines: Vec<&str> = csv.lines().collect();
        assert!(lines[0].starts_with("source_game,"), "a header row");
        assert_eq!(
            lines.len() - 1,
            state.fields.len(),
            "one row per visible field, no more and no fewer",
        );
        assert!(csv.contains("model_animation_graph"));
        for line in &lines[1..] {
            assert_eq!(
                line.matches('"').count() % 2,
                0,
                "unbalanced quoting: {line}"
            );
        }
    }

    /// Render the tab for real, against the shipped database.
    ///
    /// The state tests above prove the queries answer correctly; this proves the
    /// widget that shows them survives contact with the answers. An egui panel that
    /// panics on an empty selection, a grid whose column count disagrees with the
    /// cells pushed into it, a `SidePanel` nested somewhere it cannot go — none of
    /// that shows up until something actually lays it out, and a native GL window
    /// is not something a test can click through.
    #[test]
    fn the_tab_lays_out_against_the_shipped_database() {
        let mut state = loaded();

        let render = |state: &mut TagCompatUiState| {
            let ctx = egui::Context::default();
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(1100.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        crate::app::help::window::draw_tag_compat_body_for_tests(ui, state)
                    });
                },
            );
        };

        // Nothing selected yet — the empty state is the first thing a user sees, so
        // it has to lay out before anything is chosen.
        render(&mut state);
        assert!(state.selected_group.is_none());
        assert!(
            !state.groups.is_empty(),
            "the group list populated during the frame"
        );

        // A group selected, with rows for the grid to lay out.
        state.focus("haloreach_mcc", "haloce_evolved", "model_animation_graph");
        render(&mut state);
        assert!(!state.fields.is_empty(), "the grid had rows to lay out");

        // And the clean case, where the grid is empty and the "nothing to report"
        // message takes its place.
        state.focus("haloreach_mcc", "haloce_evolved", "sound_looping");
        state.losses_only = true;
        render(&mut state);
        assert!(state.fields.is_empty(), "sound_looping loses nothing");
    }

    /// The reverse direction is a different question with a different answer, and
    /// the window must not silently show one for the other.
    #[test]
    fn the_two_directions_disagree_about_what_is_dropped() {
        let mut forward = loaded();
        forward.focus("haloreach_mcc", "haloce_evolved", "model_animation_graph");
        forward.refresh();
        let mut backward = loaded();
        backward.focus("haloce_evolved", "haloreach_mcc", "model_animation_graph");
        backward.refresh();

        let dropped = |state: &TagCompatUiState| -> Vec<String> {
            state
                .fields
                .iter()
                .filter(|row| row.verdict == CompatVerdict::SourceOnly)
                .filter_map(|row| row.source_name.clone())
                .collect()
        };
        assert_ne!(
            dropped(&forward),
            dropped(&backward),
            "Reach loses node flags going in; Campaign Evolved loses its own additions coming back",
        );
    }

    /// A verdict this build does not know, from a database a newer generator
    /// wrote, used to read as `Identical` and show as "transfers unchanged".
    /// It is `Unknown` now, counted as a loss, and kept by the losses filter.
    #[test]
    fn an_unknown_verdict_is_never_shown_as_transferring_unchanged() {
        assert_eq!(CompatVerdict::parse("identical"), CompatVerdict::Identical);
        let unknown = CompatVerdict::parse("lossy_in_some_new_way");
        assert_eq!(unknown, CompatVerdict::Unknown);
        assert!(unknown.is_loss());
        assert_ne!(unknown.explain(), CompatVerdict::Identical.explain());

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE groups (pair_id INTEGER, group_name TEXT, verdict TEXT,
                 size_diff_structs INTEGER, source_only_fields INTEGER,
                 target_only_fields INTEGER, blocked_reason TEXT);
             INSERT INTO groups VALUES (1, 'biped', 'identical', 0, 0, 0, NULL);
             INSERT INTO groups VALUES (1, 'weapon', 'lossy_in_some_new_way', 0, 0, 0, NULL);
             INSERT INTO groups VALUES (1, 'vehicle', 'source_only', 0, 1, 0, NULL);",
            )
            .unwrap();
        let losses: Vec<(String, CompatVerdict)> = query_groups(&connection, 1, true, "")
            .unwrap()
            .into_iter()
            .map(|row| (row.group, row.verdict))
            .collect();
        assert_eq!(
            losses,
            [
                ("vehicle".to_owned(), CompatVerdict::SourceOnly),
                ("weapon".to_owned(), CompatVerdict::Unknown),
            ]
        );
        assert_eq!(query_groups(&connection, 1, false, "").unwrap().len(), 3);
    }
}
