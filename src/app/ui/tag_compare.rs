//! Source selection and results for comparing a tag with another tag.

use super::*;
use egui_extras::{Column, TableBuilder};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::Command;

fn display_path(path: &str) -> String {
    if std::path::MAIN_SEPARATOR == '\\' {
        path.replace('/', "\\")
    } else {
        path.replace('\\', "/")
    }
}

fn truncate_start(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high + 1) / 2;
        let suffix: String = chars[chars.len() - mid..].iter().collect();
        if measure(&format!("…{suffix}")) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    format!("…{}", chars[chars.len() - low..].iter().collect::<String>())
}

fn truncate_end(text: &str, max_width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high + 1) / 2;
        let prefix: String = chars[..mid].iter().collect();
        if measure(&format!("{prefix}…")) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    format!("{}…", chars[..low].iter().collect::<String>())
}

fn commit_text(commit: &GitHistoryCommit) -> String {
    format!(
        "{} · {} · {}",
        commit.short_hash, commit.date, commit.subject
    )
}

fn path_text(ui: &Ui, path: &str, width: f32, font: egui::FontId) -> String {
    let path = display_path(path);
    truncate_start(&path, width, |text| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), text_dark())
            .size()
            .x
    })
}

fn path_label(ui: &mut Ui, path: &str, width: f32) {
    let full = display_path(path);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let short = path_text(ui, &full, width - 4.0, font.clone());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, BUTTON_HEIGHT), egui::Sense::hover());
    ui.painter().text(
        egui::pos2(rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        short,
        font,
        text_dark(),
    );
    response.on_hover_text(full);
}

struct ComparisonKit {
    name: String,
    tags: PathBuf,
}

struct OpenTagGroup {
    kit: KitId,
    name: String,
    location: String,
    tags: Vec<(String, String)>,
}

fn matching_tag_path(current_key: &str, current_root: &Path, other_root: &Path) -> Option<PathBuf> {
    let current = Path::new(current_key.strip_prefix("file:")?);
    let relative = current.strip_prefix(current_root).ok()?;
    Some(other_root.join(relative))
}

fn git_relative_tag_path(kit_root: &Path, tag_path: &Path) -> Result<String, String> {
    let root_output = Command::new("git")
        .arg("-C")
        .arg(kit_root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| format!("Could not run Git: {error}"))?;
    if !root_output.status.success() {
        return Err(format!(
            "Could not find a Git repository for this editing kit: {}",
            String::from_utf8_lossy(&root_output.stderr).trim()
        ));
    }
    let repo_root = PathBuf::from(String::from_utf8_lossy(&root_output.stdout).trim());
    let repo_root = repo_root
        .canonicalize()
        .map_err(|error| format!("Could not resolve Git repository path: {error}"))?;
    let tag_path = tag_path
        .canonicalize()
        .map_err(|error| format!("Could not resolve current tag path: {error}"))?;
    let relative = tag_path
        .strip_prefix(&repo_root)
        .map_err(|_| "The current tag is outside this editing kit's Git repository.".to_owned())?;
    let relative = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or("Git cannot address a tag path containing invalid Unicode.")?
        .join("/");
    Ok(relative)
}

fn git_tag_bytes(kit_root: &Path, tag_path: &Path, revision: &str) -> Result<Vec<u8>, String> {
    if revision != "HEAD"
        && (revision.len() < 40
            || revision.len() > 64
            || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err("Invalid Git commit selection.".to_owned());
    }
    let relative = git_relative_tag_path(kit_root, tag_path)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(kit_root)
        .arg("show")
        .arg(format!("{revision}:{relative}"))
        .output()
        .map_err(|error| format!("Could not run Git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not read this tag from Git {revision}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn git_commit_parent(kit_root: &Path, revision: &str) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(kit_root)
        .args(["rev-list", "--parents", "-n", "1", revision])
        .output()
        .map_err(|error| format!("Could not run Git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not read the selected commit's parent: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_owned))
}

fn git_tag_bytes_if_present(
    kit_root: &Path,
    tag_path: &Path,
    revision: &str,
) -> Result<Option<Vec<u8>>, String> {
    let relative = git_relative_tag_path(kit_root, tag_path)?;
    let object = format!("{revision}:{relative}");
    let exists = Command::new("git")
        .arg("-C")
        .arg(kit_root)
        .args(["cat-file", "-e", &object])
        .output()
        .map_err(|error| format!("Could not run Git: {error}"))?;
    if !exists.status.success() {
        return Ok(None);
    }
    git_tag_bytes(kit_root, tag_path, revision).map(Some)
}

const GIT_HISTORY_PAGE: usize = 10;

fn git_tag_history(
    kit_root: &Path,
    tag_path: &Path,
    skip: usize,
) -> Result<(Vec<GitHistoryCommit>, bool), String> {
    let relative = git_relative_tag_path(kit_root, tag_path)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(kit_root)
        .arg("log")
        .arg(format!("--max-count={}", GIT_HISTORY_PAGE + 1))
        .arg(format!("--skip={skip}"))
        .arg("--format=%H%x09%h%x09%cs%x09%s")
        .arg("HEAD")
        .arg("--")
        .arg(format!(":(top,literal){relative}"))
        .output()
        .map_err(|error| format!("Could not run Git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not load Git history: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut commits: Vec<GitHistoryCommit> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            Some(GitHistoryCommit {
                hash: fields.next()?.to_owned(),
                short_hash: fields.next()?.to_owned(),
                date: fields.next()?.to_owned(),
                subject: fields.next()?.to_owned(),
            })
        })
        .collect();
    let has_more = commits.len() > GIT_HISTORY_PAGE;
    commits.truncate(GIT_HISTORY_PAGE);
    Ok((commits, has_more))
}

fn selected_open_tag<'a>(
    kits: &'a [Kit],
    kit: Option<KitId>,
    key: Option<&str>,
) -> Option<&'a TagFile> {
    let kit = kit?;
    let key = key?;
    kits.iter()
        .find(|candidate| candidate.id == kit)?
        .parsed_tags
        .get(key)
        .map(|document| &document.tag)
}

fn show_diff(filters: TagDiffFilters, diff: &TagFieldDiff) -> bool {
    match (diff.a.is_empty(), diff.b.is_empty()) {
        (false, false) | (true, true) => filters.both,
        (false, true) => filters.current_only,
        (true, false) => filters.comparison_only,
    }
}

fn filters_for_display(filters: TagDiffFilters, swapped: bool) -> TagDiffFilters {
    if swapped {
        TagDiffFilters {
            both: filters.both,
            current_only: filters.comparison_only,
            comparison_only: filters.current_only,
        }
    } else {
        filters
    }
}

fn comparison_results(a: &TagFile, b: &TagFile) -> TagDiffResults {
    let names = TagNameIndex::default();
    let (diffs, truncated) = diff_tags(a, b, &names, 5000);
    let (reverse_diffs, reverse_truncated) = diff_tags(b, a, &names, 5000);
    TagDiffResults {
        diffs,
        truncated,
        reverse_diffs,
        reverse_truncated,
    }
}

fn comparison_results_with_missing(
    before: Option<&TagFile>,
    after: Option<&TagFile>,
) -> TagDiffResults {
    match (before, after) {
        (Some(before), Some(after)) => comparison_results(before, after),
        (Some(tag), None) | (None, Some(tag)) => {
            let names = TagNameIndex::default();
            let (mut added, truncated) = describe_tag(tag, &names, 4999);
            added.insert(
                0,
                TagFieldDiff {
                    path: "Tag".to_owned(),
                    base_path: None,
                    a: String::new(),
                    b: "added — tag".to_owned(),
                },
            );
            let removed = added
                .iter()
                .map(|diff| TagFieldDiff {
                    path: diff.path.clone(),
                    base_path: Some(diff.path.clone()),
                    a: if diff.path == "Tag" {
                        "removed — tag".to_owned()
                    } else {
                        diff.b.clone()
                    },
                    b: String::new(),
                })
                .collect();
            let (diffs, reverse_diffs) = if before.is_some() {
                (removed, added)
            } else {
                (added, removed)
            };
            TagDiffResults {
                diffs,
                truncated,
                reverse_diffs,
                reverse_truncated: truncated,
            }
        }
        (None, None) => TagDiffResults {
            diffs: Vec::new(),
            truncated: false,
            reverse_diffs: Vec::new(),
            reverse_truncated: false,
        },
    }
}

fn displayed_results(results: &TagDiffResults, swapped: bool) -> (&[TagFieldDiff], bool) {
    if swapped {
        (&results.reverse_diffs, results.reverse_truncated)
    } else {
        (&results.diffs, results.truncated)
    }
}

fn decimal_places(value: &str) -> usize {
    let (mantissa, exponent) = value
        .split_once(['e', 'E'])
        .map(|(mantissa, exponent)| (mantissa, exponent.parse::<i32>().unwrap_or(0)))
        .unwrap_or((value, 0));
    let fraction = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len()) as i32;
    (fraction - exponent).clamp(0, 15) as usize
}

fn numeric_delta(diff: &TagFieldDiff) -> Option<(Ordering, String)> {
    let current = diff.a.trim().parse::<f64>().ok()?;
    let comparison = diff.b.trim().parse::<f64>().ok()?;
    if !current.is_finite() || !comparison.is_finite() {
        return None;
    }
    let direction = comparison
        .partial_cmp(&current)
        .filter(|order| *order != Ordering::Equal)?;
    let amount = (comparison - current).abs();
    if !amount.is_finite() {
        return None;
    }
    let precision = decimal_places(&diff.a).max(decimal_places(&diff.b));
    let mut formatted = format!("{amount:.precision$}");
    if formatted.parse::<f64>().ok() == Some(0.0) {
        formatted = format!("{amount:e}");
    }
    Some((direction, formatted))
}

fn block_change_icon(diff: &TagFieldDiff) -> Option<ButtonIcon> {
    if diff.a.is_empty() && diff.base_path.is_none() && diff.b.starts_with("added — ") {
        Some(ButtonIcon::Add)
    } else if diff.b.is_empty() && diff.base_path.is_some() && diff.a.starts_with("removed — ") {
        Some(ButtonIcon::Remove)
    } else {
        None
    }
}

impl Baboon {
    fn comparison_kits(&self, game: &str, current_root: &Path) -> Vec<ComparisonKit> {
        let mut kits = Vec::new();
        for profile in &self.custom_editing_kit_profiles {
            if profile.game != game {
                continue;
            }
            if let Ok(layout) = self.editing_kit_validation.custom(&profile.id) {
                if !same_recent_path(&layout.tags, current_root)
                    && !kits
                        .iter()
                        .any(|kit: &ComparisonKit| same_recent_path(&kit.tags, &layout.tags))
                {
                    kits.push(ComparisonKit {
                        name: profile.name.clone(),
                        tags: layout.tags,
                    });
                }
            }
        }
        for shortcut in EDITING_KIT_SHORTCUTS {
            if shortcut.game != game {
                continue;
            }
            if let Some(layout) = self.editing_kit_validation.builtin(shortcut).layout() {
                if !same_recent_path(&layout.tags, current_root)
                    && !kits
                        .iter()
                        .any(|kit| same_recent_path(&kit.tags, &layout.tags))
                {
                    kits.push(ComparisonKit {
                        name: game_display_name(game).to_owned(),
                        tags: layout.tags.clone(),
                    });
                }
            }
        }
        kits
    }

    pub(super) fn draw_tag_diff_window(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.tag_diff.take() else {
            return;
        };
        let diff_kit = self.kit_index(state.kit).unwrap_or(self.active);
        let current = self.kits[diff_kit].parsed_tags.get(&state.a_key);
        let group = current.map(|doc| doc.tag.group().tag);
        let source = self.kits[diff_kit].source.as_ref();
        let game = source.and_then(|source| source.game.as_deref());
        let (tags_root, definitions_root) = source
            .and_then(|source| match &source.source {
                TagSource::LooseFolder {
                    root,
                    definitions_root,
                    ..
                } => Some((root.as_path(), Some(definitions_root.as_path()))),
                _ => None,
            })
            .unwrap_or((Path::new(""), None));
        let git_tracked = self.kits[diff_kit]
            .profile
            .as_ref()
            .and_then(|identity| {
                self.custom_editing_kit_profiles
                    .iter()
                    .find(|profile| profile.id == identity.id)
            })
            .is_some_and(|profile| profile.git_tracked);
        let current_path = state.a_key.strip_prefix("file:").map(PathBuf::from);
        let git_available =
            git_tracked && current_path.is_some() && !tags_root.as_os_str().is_empty();
        if state.source == TagCompareSource::GitHistory
            && git_available
            && !state.git_history.loaded
        {
            state.git_history.loaded = true;
            if let Some(path) = current_path.as_ref() {
                match git_tag_history(tags_root, path, 0) {
                    Ok((commits, has_more)) => {
                        state.git_history.commits = commits;
                        state.git_history.has_more = has_more;
                        state.git_history.error = None;
                    }
                    Err(error) => state.git_history.error = Some(error),
                }
            }
        }
        let kits = game
            .filter(|_| !tags_root.as_os_str().is_empty())
            .map(|game| self.comparison_kits(game, tags_root))
            .unwrap_or_default();
        if state
            .comparison_kit_root
            .as_ref()
            .is_some_and(|root| !kits.iter().any(|kit| same_recent_path(root, &kit.tags)))
        {
            state.comparison_kit_root = None;
            state.results = None;
        }
        let current_game = current.map(|doc| blam_tags::game::Game::of(&doc.tag));
        let open_tags: Vec<OpenTagGroup> = self
            .kits
            .iter()
            .filter_map(|kit| {
                let other_source = kit.source.as_ref()?;
                if game
                    .zip(other_source.game.as_deref())
                    .is_some_and(|(a, b)| a != b)
                {
                    return None;
                }
                let mut tags: Vec<(String, String)> = kit
                    .open_tabs
                    .iter()
                    .filter_map(|key| {
                        if kit.id == state.kit && *key == state.a_key {
                            return None;
                        }
                        let doc = kit.parsed_tags.get(key)?;
                        if Some(doc.tag.group().tag) != group
                            || Some(blam_tags::game::Game::of(&doc.tag)) != current_game
                        {
                            return None;
                        }
                        let label = kit
                            .entry_for_key(key)
                            .map(|entry| entry.display_path.clone())
                            .unwrap_or_else(|| key.strip_prefix("file:").unwrap_or(key).to_owned());
                        Some((key.clone(), label))
                    })
                    .collect();
                if tags.is_empty() {
                    return None;
                }
                tags.sort_by(|a, b| a.1.cmp(&b.1));
                let name = kit
                    .profile
                    .as_ref()
                    .map(|profile| profile.name.clone())
                    .unwrap_or_else(|| display_path(&other_source.label));
                let location = display_path(&match &other_source.source {
                    TagSource::LooseFolder { root, .. } => root.display().to_string(),
                    _ => kit
                        .requested_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| other_source.label.clone()),
                });
                Some(OpenTagGroup {
                    kit: kit.id,
                    name,
                    location,
                    tags,
                })
            })
            .collect();

        let matched = state
            .comparison_kit_root
            .as_ref()
            .and_then(|root| matching_tag_path(&state.a_key, tags_root, root));
        let selected_path = match state.source {
            TagCompareSource::OpenTag => None,
            TagCompareSource::File => state.b_path.clone(),
            TagCompareSource::EditingKit => matched.clone(),
            TagCompareSource::GitHead => None,
            TagCompareSource::GitHistory => None,
        };
        let can_compare = match state.source {
            TagCompareSource::OpenTag => {
                state
                    .b_kit
                    .zip(state.b_key.as_ref())
                    .is_some_and(|(id, key)| {
                        open_tags.iter().any(|group| {
                            group.kit == id && group.tags.iter().any(|(tag, _)| tag == key)
                        })
                    })
            }
            TagCompareSource::File => selected_path.as_ref().is_some_and(|path| path.is_file()),
            TagCompareSource::EditingKit => {
                selected_path.as_ref().is_some_and(|path| path.is_file())
            }
            TagCompareSource::GitHead => git_available,
            TagCompareSource::GitHistory => {
                git_available
                    && state.git_history.selected.as_ref().is_some_and(|selected| {
                        state
                            .git_history
                            .commits
                            .iter()
                            .any(|commit| &commit.hash == selected)
                    })
            }
        } && current.is_some();
        let comparison_label = match state.source {
            TagCompareSource::OpenTag => "Open tag".to_owned(),
            TagCompareSource::File => "Selected file".to_owned(),
            TagCompareSource::EditingKit => kits
                .iter()
                .find(|kit| {
                    state
                        .comparison_kit_root
                        .as_ref()
                        .is_some_and(|root| same_recent_path(root, &kit.tags))
                })
                .map(|kit| kit.name.clone())
                .unwrap_or_else(|| "Editing kit".to_owned()),
            TagCompareSource::GitHead => "Git HEAD".to_owned(),
            TagCompareSource::GitHistory => "After".to_owned(),
        };
        let current_label = if state.source == TagCompareSource::GitHistory {
            "Before".to_owned()
        } else if state.source == TagCompareSource::EditingKit {
            self.kits[diff_kit]
                .profile
                .as_ref()
                .map(|profile| profile.name.clone())
                .or_else(|| source.map(|source| display_path(&source.label)))
                .unwrap_or_else(|| "Current tag".to_owned())
        } else {
            "Current tag".to_owned()
        };
        let mut open = true;
        let mut browse = false;
        let mut compare = false;
        let had_results = state.results.is_some();
        let mut selection_changed = false;
        egui::Window::new("Compare Tags")
            .id(egui::Id::new("tag_diff_window"))
            .title_bar(false)
            .collapsible(false)
            .default_width(620.0)
            .resizable(true)
            .show(ctx, |ui| {
                super::find::draw_icon_window_header(
                    ui,
                    "Compare Tags",
                    ButtonIcon::Compare,
                    &mut open,
                );
                ui.separator();
                let label_width = ui
                    .painter()
                    .layout_no_wrap(
                        "Compare Source:".to_owned(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        text_dark(),
                    )
                    .size()
                    .x;
                let field_width = (ui.available_width() - label_width - 22.0).max(120.0);
                let menu_path_width = (field_width - 24.0).max(40.0);
                egui::Grid::new("tag_compare_source_grid")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(if state.source == TagCompareSource::GitHistory {
                                "Tag Path:"
                            } else {
                                "Current Tag:"
                            })
                            .strong(),
                        );
                        path_label(
                            ui,
                            state.a_key.strip_prefix("file:").unwrap_or(&state.a_key),
                            field_width,
                        );
                        ui.end_row();

                        ui.label(RichText::new("Compare Source:").strong());
                        let source_name = match state.source {
                            TagCompareSource::OpenTag => "Open Tag",
                            TagCompareSource::File => "Browse for Tag",
                            TagCompareSource::EditingKit => "Matching Tag in Another Kit",
                            TagCompareSource::GitHead => "Git HEAD",
                            TagCompareSource::GitHistory => "Git History…",
                        };
                        egui::ComboBox::from_id_salt("tag_compare_source")
                            .selected_text(source_name)
                            .width(field_width)
                            .truncate()
                            .show_ui(ui, |ui| {
                                for (source, label) in [
                                    (TagCompareSource::OpenTag, "Open Tag"),
                                    (TagCompareSource::File, "Browse for Tag"),
                                    (TagCompareSource::EditingKit, "Matching Tag in Another Kit"),
                                    (TagCompareSource::GitHead, "Git HEAD"),
                                    (TagCompareSource::GitHistory, "Git History…"),
                                ] {
                                    let mut text = egui::text::LayoutJob::default();
                                    text.append(
                                        label,
                                        0.0,
                                        egui::TextFormat {
                                            font_id: egui::TextStyle::Body.resolve(ui.style()),
                                            color: text_dark(),
                                            ..Default::default()
                                        },
                                    );
                                    if source == TagCompareSource::EditingKit {
                                        text.append(
                                            "\nSame path and tag in another editing kit",
                                            0.0,
                                            egui::TextFormat {
                                                font_id: egui::TextStyle::Small.resolve(ui.style()),
                                                color: subtle_dark(),
                                                ..Default::default()
                                            },
                                        );
                                    } else if source == TagCompareSource::GitHead {
                                        text.append(
                                            "\nSame tag in the current Git commit",
                                            0.0,
                                            egui::TextFormat {
                                                font_id: egui::TextStyle::Small.resolve(ui.style()),
                                                color: subtle_dark(),
                                                ..Default::default()
                                            },
                                        );
                                    } else if source == TagCompareSource::GitHistory {
                                        text.append(
                                            "\nChanges introduced by a selected commit",
                                            0.0,
                                            egui::TextFormat {
                                                font_id: egui::TextStyle::Small.resolve(ui.style()),
                                                color: subtle_dark(),
                                                ..Default::default()
                                            },
                                        );
                                    }
                                    let changed = if matches!(
                                        source,
                                        TagCompareSource::GitHead | TagCompareSource::GitHistory
                                    ) && !git_available
                                    {
                                        ui.add_enabled_ui(false, |ui| {
                                            ui.selectable_value(&mut state.source, source, text)
                                        })
                                        .inner
                                        .changed()
                                    } else {
                                        ui.selectable_value(&mut state.source, source, text)
                                            .changed()
                                    };
                                    if changed {
                                        state.results = None;
                                        state.error = None;
                                    }
                                }
                            });
                        ui.end_row();

                        match state.source {
                            TagCompareSource::OpenTag => {
                                ui.label(RichText::new("Open Tag:").strong());
                                let selected =
                                    state.b_kit.zip(state.b_key.as_ref()).and_then(|(id, key)| {
                                        open_tags.iter().find(|group| group.kit == id).and_then(
                                            |group| {
                                                group.tags.iter().find(|(tag, _)| tag == key).map(
                                                    |(_, label)| {
                                                        let prefix = format!("{} — ", group.name);
                                                        let font = egui::TextStyle::Button
                                                            .resolve(ui.style());
                                                        let prefix_width = ui
                                                            .painter()
                                                            .layout_no_wrap(
                                                                prefix.clone(),
                                                                font.clone(),
                                                                text_dark(),
                                                            )
                                                            .size()
                                                            .x;
                                                        let path = path_text(
                                                            ui,
                                                            label,
                                                            (field_width - 40.0 - prefix_width)
                                                                .max(20.0),
                                                            font,
                                                        );
                                                        format!("{prefix}{path}")
                                                    },
                                                )
                                            },
                                        )
                                    });
                                egui::ComboBox::from_id_salt("tag_diff_open_tag")
                                    .selected_text(
                                        selected.as_deref().unwrap_or("Select an open tag"),
                                    )
                                    .width(field_width)
                                    .truncate()
                                    .show_ui(ui, |ui| {
                                        for (index, group) in open_tags.iter().enumerate() {
                                            if index > 0 {
                                                ui.separator();
                                            }
                                            ui.label(RichText::new(&group.name).strong())
                                                .on_hover_text(&group.location);
                                            let location = path_text(
                                                ui,
                                                &group.location,
                                                menu_path_width,
                                                egui::TextStyle::Small.resolve(ui.style()),
                                            );
                                            ui.label(
                                                RichText::new(location)
                                                    .small()
                                                    .color(subtle_dark()),
                                            )
                                            .on_hover_text(&group.location);
                                            for (key, label) in &group.tags {
                                                let selected = state.b_kit == Some(group.kit)
                                                    && state.b_key.as_ref() == Some(key);
                                                let shown = path_text(
                                                    ui,
                                                    label,
                                                    menu_path_width,
                                                    egui::TextStyle::Body.resolve(ui.style()),
                                                );
                                                if ui
                                                    .selectable_label(selected, shown)
                                                    .on_hover_text(display_path(label))
                                                    .clicked()
                                                {
                                                    state.b_kit = Some(group.kit);
                                                    state.b_key = Some(key.clone());
                                                    state.results = None;
                                                    state.error = None;
                                                    selection_changed = true;
                                                }
                                            }
                                        }
                                    });
                                ui.end_row();
                            }
                            TagCompareSource::File => {
                                ui.label(RichText::new("Compare With:").strong());
                                ui.horizontal(|ui| {
                                    let path = state
                                        .b_path
                                        .as_ref()
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_else(|| "No tag selected".to_owned());
                                    path_label(ui, &path, (field_width - 90.0).max(80.0));
                                    if ui.button("Browse…").clicked() {
                                        browse = true;
                                    }
                                });
                                ui.end_row();
                            }
                            TagCompareSource::EditingKit => {
                                ui.label(RichText::new("Editing Kit:").strong());
                                let chosen = kits.iter().find(|kit| {
                                    state
                                        .comparison_kit_root
                                        .as_ref()
                                        .is_some_and(|root| same_recent_path(root, &kit.tags))
                                });
                                egui::ComboBox::from_id_salt("tag_compare_kit")
                                    .selected_text(
                                        chosen
                                            .map(|kit| kit.name.as_str())
                                            .unwrap_or("Select an editing kit"),
                                    )
                                    .width(field_width)
                                    .truncate()
                                    .show_ui(ui, |ui| {
                                        for kit in &kits {
                                            let selected =
                                                state.comparison_kit_root.as_ref().is_some_and(
                                                    |root| same_recent_path(root, &kit.tags),
                                                );
                                            if ui
                                                .selectable_label(selected, &kit.name)
                                                .on_hover_text(display_path(
                                                    &kit.tags.display().to_string(),
                                                ))
                                                .clicked()
                                            {
                                                state.comparison_kit_root = Some(kit.tags.clone());
                                                state.results = None;
                                                state.error = None;
                                                selection_changed = true;
                                            }
                                        }
                                    });
                                ui.end_row();
                                ui.label(RichText::new("Compare With:").strong());
                                let path = matched
                                    .as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_else(|| "Select an editing kit".to_owned());
                                path_label(ui, &path, field_width);
                                ui.end_row();
                            }
                            TagCompareSource::GitHead => {
                                ui.label(RichText::new("Compare With:").strong());
                                let label = current_path
                                    .as_ref()
                                    .map(|path| format!("HEAD: {}", path.display()))
                                    .unwrap_or_else(|| "No loose tag selected".to_owned());
                                path_label(ui, &label, field_width);
                                ui.end_row();
                            }
                            TagCompareSource::GitHistory => {
                                ui.label(RichText::new("Commit:").strong());
                                let selected = state
                                    .git_history
                                    .selected
                                    .as_ref()
                                    .and_then(|hash| {
                                        state
                                            .git_history
                                            .commits
                                            .iter()
                                            .find(|commit| &commit.hash == hash)
                                    })
                                    .map(commit_text)
                                    .unwrap_or_else(|| "Select a commit".to_owned());
                                egui::ComboBox::from_id_salt("tag_compare_git_history")
                                    .selected_text(selected)
                                    .width(field_width)
                                    .truncate()
                                    .show_ui(ui, |ui| {
                                        for commit in &state.git_history.commits {
                                            let selected = state.git_history.selected.as_deref()
                                                == Some(commit.hash.as_str());
                                            let label = commit_text(commit);
                                            let font = egui::TextStyle::Button.resolve(ui.style());
                                            let shown =
                                                truncate_end(&label, menu_path_width, |text| {
                                                    ui.painter()
                                                        .layout_no_wrap(
                                                            text.to_owned(),
                                                            font.clone(),
                                                            text_dark(),
                                                        )
                                                        .size()
                                                        .x
                                                });
                                            if ui
                                                .selectable_label(selected, shown)
                                                .on_hover_text(format!("{label}\n{}", commit.hash))
                                                .clicked()
                                            {
                                                state.git_history.selected =
                                                    Some(commit.hash.clone());
                                                state.results = None;
                                                state.error = None;
                                                selection_changed = true;
                                            }
                                        }
                                        if state.git_history.has_more {
                                            ui.separator();
                                            if ui.button("Load older commits…").clicked() {
                                                let skip = state.git_history.commits.len();
                                                if let Some(path) = current_path.as_ref() {
                                                    match git_tag_history(tags_root, path, skip) {
                                                        Ok((commits, has_more)) => {
                                                            state
                                                                .git_history
                                                                .commits
                                                                .extend(commits);
                                                            state.git_history.has_more = has_more;
                                                            state.git_history.error = None;
                                                        }
                                                        Err(error) => {
                                                            state.git_history.error = Some(error)
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    });
                                ui.end_row();
                                ui.label(RichText::new("Comparing:").strong());
                                ui.label("Before commit → After commit");
                                ui.end_row();
                            }
                        }
                    });
                if state.source == TagCompareSource::OpenTag && open_tags.is_empty() {
                    ui.label(
                        RichText::new("No other open tags of this type in a matching engine.")
                            .color(subtle_dark()),
                    );
                }
                if state.source == TagCompareSource::EditingKit {
                    if tags_root.as_os_str().is_empty() || !state.a_key.starts_with("file:") {
                        ui.label(
                            RichText::new("Matching tags require a loose editing kit tag.")
                                .color(subtle_dark()),
                        );
                    } else if kits.is_empty() {
                        ui.label(
                            RichText::new("No other configured editing kit uses this engine.")
                                .color(subtle_dark()),
                        );
                    } else if let Some(path) = &matched {
                        if !path.is_file() {
                            ui.label(
                                RichText::new("No matching tag at this path and tag type.")
                                    .color(subtle_dark()),
                            );
                        }
                    }
                }
                if matches!(
                    state.source,
                    TagCompareSource::GitHead | TagCompareSource::GitHistory
                ) && !git_available
                {
                    ui.label(
                        RichText::new(if git_tracked {
                            "Git comparison requires a loose editing kit tag."
                        } else {
                            "Enable Tracked in Git in this editing kit's settings."
                        })
                        .color(subtle_dark()),
                    );
                }
                if state.source == TagCompareSource::GitHistory {
                    if let Some(error) = &state.git_history.error {
                        ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                    } else if state.git_history.loaded && state.git_history.commits.is_empty() {
                        ui.label(
                            RichText::new("No commits have changed this tag at its current path.")
                                .color(subtle_dark()),
                        );
                    }
                }
                if let Some(error) = &state.error {
                    ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        compare = icon_text_button(ui, ButtonIcon::Compare, "Compare", can_compare)
                            .clicked();
                    });
                });

                if let Some(results) = &state.results {
                    ui.separator();
                    if displayed_results(results, state.swapped).0.is_empty() {
                        ui.label(RichText::new("No differences.").color(subtle_dark()));
                    } else {
                        ui.horizontal(|ui| {
                            icon_text_dropdown_button(ui, ButtonIcon::Filter, "Filter", |ui| {
                                let history = state.source == TagCompareSource::GitHistory;
                                ui.checkbox(
                                    &mut state.filters.both,
                                    if history {
                                        "Both versions"
                                    } else {
                                        "Both tags"
                                    },
                                );
                                ui.checkbox(
                                    &mut state.filters.current_only,
                                    if history {
                                        "Before only"
                                    } else {
                                        "Current tag only"
                                    },
                                );
                                ui.checkbox(
                                    &mut state.filters.comparison_only,
                                    if history {
                                        "After only"
                                    } else {
                                        "Comparison tag only"
                                    },
                                );
                            });
                            if icon_text_button(ui, ButtonIcon::Swap, "Swap", true)
                                .on_hover_text("Swap the two sides of the comparison")
                                .clicked()
                            {
                                state.swapped = !state.swapped;
                            }
                            let (diffs, truncated) = displayed_results(results, state.swapped);
                            let display_filters = filters_for_display(state.filters, state.swapped);
                            let visible: Vec<&TagFieldDiff> = diffs
                                .iter()
                                .filter(|diff| show_diff(display_filters, diff))
                                .collect();
                            let count = if visible.len() == diffs.len() {
                                format!("{} differing field(s)", visible.len())
                            } else {
                                format!("{} of {} differing field(s)", visible.len(), diffs.len())
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{}{}",
                                    count,
                                    if truncated { " (capped)" } else { "" }
                                ))
                                .small()
                                .color(subtle_dark()),
                            );
                            if icon_text_button(ui, ButtonIcon::Copy, "Copy", !visible.is_empty())
                                .on_hover_text("Copy the diff as tab-separated rows")
                                .clicked()
                            {
                                let (left_label, right_label) = if state.swapped {
                                    (&comparison_label, &current_label)
                                } else {
                                    (&current_label, &comparison_label)
                                };
                                let text =
                                    std::iter::once(format!("field\t{left_label}\t{right_label}"))
                                        .chain(
                                            visible
                                                .iter()
                                                .map(|d| format!("{}\t{}\t{}", d.path, d.a, d.b)),
                                        )
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                ui.output_mut(|output| output.copied_text = text);
                            }
                        });
                        ui.separator();
                        let (diffs, _) = displayed_results(results, state.swapped);
                        let display_filters = filters_for_display(state.filters, state.swapped);
                        let (left_label, right_label) = if state.swapped {
                            (&comparison_label, &current_label)
                        } else {
                            (&current_label, &comparison_label)
                        };
                        let visible: Vec<&TagFieldDiff> = diffs
                            .iter()
                            .filter(|diff| show_diff(display_filters, diff))
                            .collect();
                        if visible.is_empty() {
                            ui.label(
                                RichText::new("No differences match these filters.")
                                    .color(subtle_dark()),
                            );
                        }
                        ui.scope(|ui| {
                            ui.visuals_mut().widgets.noninteractive.bg_stroke =
                                Stroke::new(1.0_f32, foundation_group_edge());
                            let width = ui.available_width();
                            let mut header_rect: Option<egui::Rect> = None;
                            TableBuilder::new(ui)
                                .id_salt("tag_diff_table")
                                .striped(true)
                                .resizable(true)
                                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                                .max_scroll_height(460.0)
                                .min_scrolled_height(0.0)
                                .column(Column::initial(width * 0.36).at_least(100.0).clip(true))
                                .column(Column::initial(width * 0.28).at_least(80.0).clip(true))
                                .column(Column::initial(96.0).at_least(55.0).clip(true))
                                .column(Column::remainder().at_least(80.0).clip(true))
                                .header(25.0, |mut header| {
                                    for title in ["Field", left_label] {
                                        header.col(|ui| {
                                            let rect = ui.max_rect();
                                            header_rect = Some(
                                                header_rect.map_or(rect, |seen| seen.union(rect)),
                                            );
                                            ui.painter().with_clip_rect(rect).text(
                                                egui::pos2(rect.left() + 8.0, rect.center().y),
                                                egui::Align2::LEFT_CENTER,
                                                title,
                                                bold_font(14.0),
                                                text_dark(),
                                            );
                                        });
                                    }
                                    header.col(|ui| {
                                        let rect = ui.max_rect();
                                        header_rect =
                                            Some(header_rect.map_or(rect, |seen| seen.union(rect)));
                                        let font = bold_font(14.0);
                                        let text_width = ui
                                            .painter()
                                            .layout_no_wrap(
                                                "Diff.".to_owned(),
                                                font.clone(),
                                                text_dark(),
                                            )
                                            .size()
                                            .x;
                                        ui.add_space(
                                            ((ui.available_width() - text_width) / 2.0).max(0.0),
                                        );
                                        ui.add(egui::Label::new(RichText::new("Diff.").font(font)));
                                    });
                                    header.col(|ui| {
                                        let rect = ui.max_rect();
                                        header_rect =
                                            Some(header_rect.map_or(rect, |seen| seen.union(rect)));
                                        ui.painter().with_clip_rect(rect).text(
                                            egui::pos2(rect.left() + 8.0, rect.center().y),
                                            egui::Align2::LEFT_CENTER,
                                            right_label,
                                            bold_font(14.0),
                                            text_dark(),
                                        );
                                    });
                                })
                                .body(|body| {
                                    body.rows(24.0, visible.len(), |mut row| {
                                        let diff = visible[row.index()];
                                        row.col(|ui| {
                                            ui.add_space(8.0);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&diff.path).monospace().small(),
                                                )
                                                .truncate()
                                                .halign(egui::Align::Min),
                                            );
                                        });
                                        row.col(|ui| {
                                            ui.add_space(8.0);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&diff.a).color(text_dark()),
                                                )
                                                .truncate()
                                                .halign(egui::Align::Min),
                                            );
                                        });
                                        row.col(|ui| {
                                            let rect = ui.max_rect();
                                            if let Some(icon) = block_change_icon(diff) {
                                                let icon_rect = egui::Rect::from_center_size(
                                                    rect.center(),
                                                    Vec2::splat(16.0),
                                                );
                                                paint_button_icon_at(
                                                    ui,
                                                    icon,
                                                    icon_rect,
                                                    text_dark(),
                                                );
                                                ui.interact(
                                                    rect,
                                                    ui.id().with("delta"),
                                                    egui::Sense::hover(),
                                                )
                                                .on_hover_text(format!(
                                                    "{} in {right_label}",
                                                    if icon == ButtonIcon::Add {
                                                        "Added"
                                                    } else {
                                                        "Removed"
                                                    }
                                                ));
                                            } else if let Some((direction, amount)) =
                                                numeric_delta(diff)
                                            {
                                                let (arrow, color, description) = match direction {
                                                    Ordering::Greater => {
                                                        ("▲", good_news(), "Increased")
                                                    }
                                                    Ordering::Less => {
                                                        ("▼", material_delete_text(), "Decreased")
                                                    }
                                                    Ordering::Equal => unreachable!(),
                                                };
                                                ui.painter().text(
                                                    rect.center(),
                                                    egui::Align2::CENTER_CENTER,
                                                    format!("{arrow} {amount}"),
                                                    egui::TextStyle::Body.resolve(ui.style()),
                                                    color,
                                                );
                                                ui.interact(
                                                    rect,
                                                    ui.id().with("delta"),
                                                    egui::Sense::hover(),
                                                )
                                                .on_hover_text(format!(
                                                    "{description} in {right_label} by {amount}"
                                                ));
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.add_space(8.0);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&diff.b).color(text_dark()),
                                                )
                                                .truncate()
                                                .halign(egui::Align::Min),
                                            );
                                        });
                                    });
                                });
                            if let Some(rect) = header_rect {
                                ui.painter().line_segment(
                                    [rect.left_bottom(), rect.right_bottom()],
                                    Stroke::new(1.0_f32, foundation_group_edge()),
                                );
                            }
                        });
                    }
                }
            });

        if browse {
            if let Some(group) = group {
                let ext = group_tag_to_extension(group).unwrap_or("");
                let mut dialog = rfd::FileDialog::new().set_title("Select tag to compare");
                if !ext.is_empty() {
                    dialog = dialog.add_filter(ext, &[ext]);
                }
                if !tags_root.as_os_str().is_empty() {
                    dialog = dialog.set_directory(tags_root);
                }
                if let Some(path) = dialog.pick_file() {
                    state.b_path = Some(path);
                    state.results = None;
                    state.error = None;
                    selection_changed = true;
                }
            }
        }
        compare |= had_results && selection_changed;
        if compare {
            state.results = None;
            let selected_path = match state.source {
                TagCompareSource::File => state.b_path.clone(),
                TagCompareSource::EditingKit => state
                    .comparison_kit_root
                    .as_ref()
                    .and_then(|root| matching_tag_path(&state.a_key, tags_root, root)),
                _ => selected_path,
            };
            let a = self.kits[diff_kit].parsed_tags.get(&state.a_key);
            let b = match state.source {
                TagCompareSource::OpenTag => {
                    selected_open_tag(&self.kits, state.b_kit, state.b_key.as_deref())
                }
                _ => None,
            };
            if let (Some(a), Some(b)) = (a, b) {
                state.results = Some(comparison_results(&a.tag, b));
                state.error = None;
            } else if state.source == TagCompareSource::GitHead {
                if let (Some(a), Some(path), Some(group)) = (a, current_path.as_ref(), group) {
                    let result = git_tag_bytes(tags_root, path, "HEAD").and_then(|bytes| {
                        crate::source::read_tag_from_bytes(&bytes, game, definitions_root, group)
                            .map_err(|error| format!("Could not load tag from Git HEAD: {error}"))
                    });
                    match result {
                        Ok(b) if b.group().tag == group => {
                            state.results = Some(comparison_results(&a.tag, &b));
                            state.error = None;
                        }
                        Ok(_) => {
                            state.error =
                                Some("The tag in Git HEAD has a different tag type.".to_owned())
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
            } else if state.source == TagCompareSource::GitHistory {
                if let (Some(path), Some(group), Some(revision)) = (
                    current_path.as_ref(),
                    group,
                    state.git_history.selected.as_deref(),
                ) {
                    let load_tag = |revision: &str| -> Result<Option<TagFile>, String> {
                        git_tag_bytes_if_present(tags_root, path, revision)?
                            .map(|bytes| {
                                let tag = crate::source::read_tag_from_bytes(
                                    &bytes,
                                    game,
                                    definitions_root,
                                    group,
                                )
                                .map_err(|error| {
                                    format!("Could not load tag from commit {revision}: {error}")
                                })?;
                                if tag.group().tag != group {
                                    return Err(format!(
                                        "The tag in commit {revision} has a different tag type."
                                    ));
                                }
                                Ok(tag)
                            })
                            .transpose()
                    };
                    let result = git_commit_parent(tags_root, revision).and_then(|parent| {
                        let before = parent.as_deref().map(&load_tag).transpose()?.flatten();
                        let after = load_tag(revision)?;
                        Ok(comparison_results_with_missing(
                            before.as_ref(),
                            after.as_ref(),
                        ))
                    });
                    match result {
                        Ok(results) => {
                            state.results = Some(results);
                            state.error = None;
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
            } else if let (Some(a), Some(group), Some(path)) = (a, group, selected_path) {
                match crate::source::read_tag_at_path(&path, game, definitions_root, group) {
                    Ok(b) if b.group().tag == group => {
                        state.results = Some(comparison_results(&a.tag, &b));
                        state.error = None;
                    }
                    Ok(_) => {
                        state.error = Some("The selected tag has a different tag type.".to_owned())
                    }
                    Err(error) => {
                        state.error = Some(format!("Could not load comparison tag: {error}"))
                    }
                }
            }
            ctx.request_repaint();
        }
        if open {
            self.tag_diff = Some(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_head_reads_committed_tag_instead_of_working_copy() {
        let root = std::env::temp_dir().join(format!("baboon-git-head-{}", uuid::Uuid::new_v4()));
        let kit = root.join("kit");
        let tag = kit.join("tags").join("objects").join("example.model");
        std::fs::create_dir_all(tag.parent().unwrap()).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        std::fs::write(&tag, b"committed tag").unwrap();
        git(&["add", "kit/tags/objects/example.model"]);
        git(&[
            "-c",
            "user.name=Baboon Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "test tag",
        ]);
        std::fs::write(&tag, b"edited tag").unwrap();
        assert_eq!(
            git_tag_bytes(&kit.join("tags"), &tag, "HEAD").unwrap(),
            b"committed tag"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_history_pages_only_commits_that_changed_the_tag() {
        let root =
            std::env::temp_dir().join(format!("baboon-git-history-{}", uuid::Uuid::new_v4()));
        let tag = root.join("kit/tags/objects/example.model");
        std::fs::create_dir_all(tag.parent().unwrap()).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        for index in 0..12 {
            std::fs::write(&tag, format!("version {index}")).unwrap();
            git(&["add", "kit/tags/objects/example.model"]);
            git(&[
                "-c",
                "user.name=Baboon Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                &format!("Tag revision {index}"),
            ]);
        }
        std::fs::write(root.join("unrelated.txt"), b"unrelated").unwrap();
        git(&["add", "unrelated.txt"]);
        git(&[
            "-c",
            "user.name=Baboon Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "Unrelated change",
        ]);
        let tags_root = root.join("kit/tags");
        std::fs::write(&tag, b"uncommitted local version").unwrap();
        let (first, has_more) = git_tag_history(&tags_root, &tag, 0).unwrap();
        assert_eq!(first.len(), 10);
        assert!(has_more);
        assert_eq!(first[0].subject, "Tag revision 11");
        let latest_parent = git_commit_parent(&tags_root, &first[0].hash)
            .unwrap()
            .unwrap();
        assert_eq!(
            git_tag_bytes_if_present(&tags_root, &tag, &latest_parent).unwrap(),
            Some(b"version 10".to_vec())
        );
        assert_eq!(
            git_tag_bytes_if_present(&tags_root, &tag, &first[0].hash).unwrap(),
            Some(b"version 11".to_vec())
        );
        assert_eq!(first[9].subject, "Tag revision 2");
        let (older, has_more) = git_tag_history(&tags_root, &tag, first.len()).unwrap();
        assert_eq!(older.len(), 2);
        assert!(!has_more);
        assert_eq!(older[0].subject, "Tag revision 1");
        assert_eq!(older[1].subject, "Tag revision 0");
        assert_eq!(
            git_tag_bytes(&tags_root, &tag, &older[0].hash).unwrap(),
            b"version 1"
        );
        let parent = git_commit_parent(&tags_root, &older[0].hash)
            .unwrap()
            .unwrap();
        assert_eq!(
            git_tag_bytes_if_present(&tags_root, &tag, &parent).unwrap(),
            Some(b"version 0".to_vec())
        );
        assert_eq!(git_commit_parent(&tags_root, &older[1].hash).unwrap(), None);
        assert_eq!(
            git_tag_bytes_if_present(&tags_root, &tag, &older[1].hash).unwrap(),
            Some(b"version 0".to_vec())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_tag_is_shown_as_added_or_removed() {
        let tag = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        let added = comparison_results_with_missing(None, Some(&tag));
        assert_eq!(added.diffs[0].b, "added — tag");
        assert_eq!(block_change_icon(&added.diffs[0]), Some(ButtonIcon::Add));
        assert_eq!(added.reverse_diffs[0].a, "removed — tag");
        assert_eq!(
            block_change_icon(&added.reverse_diffs[0]),
            Some(ButtonIcon::Remove)
        );

        let removed = comparison_results_with_missing(Some(&tag), None);
        assert_eq!(removed.diffs[0].a, "removed — tag");
        assert_eq!(
            block_change_icon(&removed.diffs[0]),
            Some(ButtonIcon::Remove)
        );
    }

    #[test]
    fn displayed_paths_use_the_platform_separator_without_changing_keys() {
        let path = r"C:\kit/tags\objects/example.model";
        let shown = display_path(path);
        assert_eq!(
            shown,
            path.replace(['\\', '/'], &std::path::MAIN_SEPARATOR.to_string())
        );
        assert_eq!(path, r"C:\kit/tags\objects/example.model");
    }

    #[test]
    fn long_paths_keep_the_filename_visible() {
        let full = "C:/long/folder/example.model";
        let shown = truncate_start(full, 18.0, |text| text.chars().count() as f32);
        assert!(shown.starts_with('…'));
        assert!(shown.ends_with("example.model"));
        assert!(shown.chars().count() as f32 <= 18.0);
        assert_eq!(
            truncate_start(full, 40.0, |text| text.chars().count() as f32),
            full
        );
    }

    #[test]
    fn matching_tag_keeps_path_and_type_below_tags_root() {
        let base = std::env::temp_dir();
        let current_root = base.join("H2R").join("tags");
        let reference_root = base.join("H2EK").join("tags");
        let relative = Path::new("objects")
            .join("characters")
            .join("brute")
            .join("brute.model");
        let key = format!("file:{}", current_root.join(&relative).display());
        assert_eq!(
            matching_tag_path(&key, &current_root, &reference_root),
            Some(reference_root.join(relative))
        );
        let elsewhere = format!(
            "file:{}",
            base.join("Elsewhere").join("brute.model").display()
        );
        assert_eq!(
            matching_tag_path(&elsewhere, &current_root, &reference_root),
            None
        );
    }

    #[test]
    fn open_tag_selection_uses_its_owning_kit() {
        let key = "same-key".to_owned();
        let mut first = Kit::empty(KitId(1), Default::default());
        let mut second = Kit::empty(KitId(2), Default::default());
        first.parsed_tags.insert(
            key.clone(),
            TagDocument::clean(TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap()),
        );
        second.parsed_tags.insert(
            key.clone(),
            TagDocument::clean(TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap()),
        );
        let kits = [first, second];
        let selected = selected_open_tag(&kits, Some(KitId(2)), Some(&key)).unwrap();
        assert!(std::ptr::eq(selected, &kits[1].parsed_tags[&key].tag));
        assert!(selected_open_tag(&kits, Some(KitId(3)), Some(&key)).is_none());
    }

    #[test]
    fn diff_filters_follow_which_columns_have_values() {
        let row = |a: &str, b: &str| TagFieldDiff {
            path: "field".to_owned(),
            base_path: None,
            a: a.to_owned(),
            b: b.to_owned(),
        };
        let mut filters = TagDiffFilters::default();
        assert!(show_diff(filters, &row("old", "new")));
        assert!(show_diff(filters, &row("old", "")));
        assert!(show_diff(filters, &row("", "new")));

        filters.both = false;
        assert!(!show_diff(filters, &row("old", "new")));
        assert!(show_diff(filters, &row("old", "")));
        assert!(show_diff(filters, &row("", "new")));

        filters.current_only = false;
        assert!(!show_diff(filters, &row("old", "")));
        assert!(show_diff(filters, &row("", "new")));
        filters.comparison_only = false;
        assert!(!show_diff(filters, &row("", "new")));
    }

    #[test]
    fn numeric_delta_shows_precise_finite_numeric_changes() {
        let row = |a: &str, b: &str| TagFieldDiff {
            path: "field".to_owned(),
            base_path: None,
            a: a.to_owned(),
            b: b.to_owned(),
        };
        assert_eq!(
            numeric_delta(&row("3", "4.5")),
            Some((Ordering::Greater, "1.5".to_owned()))
        );
        assert_eq!(
            numeric_delta(&row("-2", "-3")),
            Some((Ordering::Less, "1".to_owned()))
        );
        assert_eq!(
            numeric_delta(&row("0.2", "0.3")),
            Some((Ordering::Greater, "0.1".to_owned()))
        );
        assert_eq!(
            numeric_delta(&row("1e-3", "2e-3")),
            Some((Ordering::Greater, "0.001".to_owned()))
        );
        assert_eq!(
            numeric_delta(&row("0", "1e-16")),
            Some((Ordering::Greater, "1e-16".to_owned()))
        );
        assert_eq!(numeric_delta(&row("1e2", "100")), None);
        assert_eq!(numeric_delta(&row("", "3")), None);
        assert_eq!(numeric_delta(&row("1", "NaN")), None);
        assert_eq!(numeric_delta(&row("1 red", "2 blue")), None);
    }

    #[test]
    fn block_change_icons_only_mark_added_or_removed_elements() {
        let row = |base_path: Option<&str>, a: &str, b: &str| TagFieldDiff {
            path: "variants[0]".to_owned(),
            base_path: base_path.map(str::to_owned),
            a: a.to_owned(),
            b: b.to_owned(),
        };
        assert_eq!(
            block_change_icon(&row(None, "", "added — variant")),
            Some(ButtonIcon::Add)
        );
        assert_eq!(
            block_change_icon(&row(Some("variants[0]"), "removed — variant", "")),
            Some(ButtonIcon::Remove)
        );
        assert_eq!(block_change_icon(&row(None, "", "field value")), None);
        assert_eq!(
            block_change_icon(&row(Some("variants[0]"), "variant", "variant")),
            None
        );
    }

    #[test]
    fn swap_recomputes_block_changes_in_the_opposite_direction() {
        let a = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        let mut b = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        crate::app::add_block_element(&mut b, "sound classes").unwrap();
        let results = comparison_results(&a, &b);
        assert!(
            results
                .diffs
                .iter()
                .any(|diff| block_change_icon(diff) == Some(ButtonIcon::Add))
        );
        assert!(
            results
                .reverse_diffs
                .iter()
                .any(|diff| block_change_icon(diff) == Some(ButtonIcon::Remove))
        );
    }

    #[test]
    fn swap_keeps_one_sided_filters_tied_to_the_same_tag() {
        let filters = TagDiffFilters {
            both: false,
            current_only: true,
            comparison_only: false,
        };
        let swapped = filters_for_display(filters, true);
        assert!(!swapped.current_only);
        assert!(swapped.comparison_only);
        let reverse_row = TagFieldDiff {
            path: "field".to_owned(),
            base_path: None,
            a: String::new(),
            b: "current value".to_owned(),
        };
        assert!(show_diff(swapped, &reverse_row));
        assert_eq!(
            numeric_delta(&TagFieldDiff {
                a: "40".to_owned(),
                b: "70".to_owned(),
                ..reverse_row
            }),
            Some((Ordering::Greater, "30".to_owned()))
        );
    }
}
