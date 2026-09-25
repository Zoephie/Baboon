//! The Git Review pane: tag-only working-tree changes and commit history.

use super::*;

enum GitReviewAction {
    Refresh,
    OpenGitHubDesktop,
    OpenRepositoryFolder,
    SelectRevision(GitReviewSelection),
    SelectFile(String),
    OpenFile(String),
}

fn repository_folder_button_label() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "File Explorer"
    }
    #[cfg(target_os = "macos")]
    {
        "Finder"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "File Manager"
    }
}

fn git_review_header_actions(
    ui: &mut Ui,
    has_repo: bool,
    has_github_desktop: bool,
    action: &mut Option<GitReviewAction>,
) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = PANE_HEADER_ACTION_GAP;
        if icon_text_button(
            ui,
            ButtonIcon::Refresh,
            "Refresh Changes & Commits",
            has_repo,
        )
        .clicked()
        {
            *action = Some(GitReviewAction::Refresh);
        }
        ui.add_space(PANE_HEADER_SECTION_GAP);
        if icon_text_button(
            ui,
            ButtonIcon::FileExplorer,
            repository_folder_button_label(),
            has_repo,
        )
        .clicked()
        {
            *action = Some(GitReviewAction::OpenRepositoryFolder);
        }
        if has_github_desktop
            && icon_text_button(ui, ButtonIcon::GitHub, "GitHub Desktop", has_repo).clicked()
        {
            *action = Some(GitReviewAction::OpenGitHubDesktop);
        }
        ui.label(RichText::new("Open In:").color(subtle_dark()));
    });
}

#[derive(Clone)]
struct GitHubDesktopLauncher {
    program: PathBuf,
    arguments: Vec<&'static str>,
}

impl GitHubDesktopLauncher {
    fn open(&self, repo: &Path) -> std::io::Result<()> {
        Command::new(&self.program)
            .args(&self.arguments)
            .arg(repo)
            .spawn()
            .map(|_| ())
    }
}

fn github_desktop_launcher() -> Option<GitHubDesktopLauncher> {
    #[cfg(target_os = "windows")]
    {
        // GitHub Desktop's stable per-user executable forwards to its current
        // version. Launch it directly rather than passing a repo path through
        // its batch wrapper (which would involve cmd.exe quoting).
        let executable = PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
            .join("GitHubDesktop")
            .join("GitHubDesktop.exe");
        return executable.is_file().then_some(GitHubDesktopLauncher {
            program: executable,
            arguments: vec!["--cli-open"],
        });
    }
    #[cfg(target_os = "macos")]
    {
        let system_app = Path::new("/Applications/GitHub Desktop.app");
        let user_app = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Applications/GitHub Desktop.app"));
        if system_app.is_dir() || user_app.is_some_and(|app| app.is_dir()) {
            return Some(GitHubDesktopLauncher {
                program: PathBuf::from("open"),
                arguments: vec!["-a", "GitHub Desktop"],
            });
        }
        return std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .map(|directory| directory.join("github"))
            .find(|candidate| candidate.is_file())
            .map(|program| GitHubDesktopLauncher {
                program,
                arguments: Vec::new(),
            });
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        None
    }
}

fn change_icon(status: &str) -> ButtonIcon {
    match status.chars().next() {
        Some('A' | '?') => ButtonIcon::ChangeAdded,
        Some('D') => ButtonIcon::ChangeRemoved,
        Some('M' | 'R' | 'C' | 'T' | 'U') => ButtonIcon::ChangeModified,
        _ => ButtonIcon::ChangeSame,
    }
}

#[derive(Default)]
struct ChangeCounts {
    added: usize,
    modified: usize,
    removed: usize,
}

fn change_counts<'a>(files: impl IntoIterator<Item = &'a GitReviewFile>) -> ChangeCounts {
    let mut counts = ChangeCounts::default();
    for file in files {
        match file.status.chars().next() {
            Some('A' | '?') => counts.added += 1,
            Some('D') => counts.removed += 1,
            Some('M' | 'R' | 'C' | 'T' | 'U') => counts.modified += 1,
            _ => {}
        }
    }
    counts
}

fn selected_revision_title(selection: &GitReviewSelection, commits: &[GitReviewCommit]) -> String {
    match selection {
        GitReviewSelection::Local => "Local Changes".to_owned(),
        GitReviewSelection::Commit(hash) => commits
            .iter()
            .find(|commit| &commit.hash == hash)
            .map(|commit| commit.subject.clone())
            .unwrap_or_else(|| format!("Commit {}", &hash[..hash.len().min(7)])),
    }
}

fn pane_title(ui: &mut Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), BUTTON_HEIGHT),
        Sense::hover(),
    );
    let font = bold_font(12.0);
    let shown = truncate_end_to_width(ui, title, &font, text_dark(), rect.width());
    ui.painter().with_clip_rect(rect).text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        shown,
        font,
        text_dark(),
    );
}

fn compare_pane_title(ui: &mut Ui, path: Option<&str>) {
    let Some(path) = path else {
        pane_title(ui, "Compare - Select a changed tag");
        return;
    };
    let path = native_git_display_path(path);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), BUTTON_HEIGHT),
        Sense::hover(),
    );
    let font = bold_font(12.0);
    let prefix = "Compare - ";
    let prefix_width = ui
        .painter()
        .layout_no_wrap(prefix.to_owned(), font.clone(), text_dark())
        .size()
        .x;
    let shown_path = truncate_start_to_width(
        ui,
        &path,
        &font,
        text_dark(),
        (rect.width() - prefix_width).max(0.0),
    );
    ui.painter().with_clip_rect(rect).text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        format!("{prefix}{shown_path}"),
        font,
        text_dark(),
    );
}

fn change_list_header(
    ui: &mut Ui,
    rect: egui::Rect,
    files: &[&GitReviewFile],
    total_count: usize,
    filtered: bool,
    revision_title: &str,
) {
    let counts = change_counts(files.iter().copied());
    let font = bold_font(12.0);
    let text_width = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), text_dark())
            .size()
            .x
    };
    let mut count_title = if filtered {
        format!("{} of {total_count} changed tags", files.len())
    } else {
        format!("{} changed tags", files.len())
    };
    let mut statuses = Vec::new();
    if counts.added > 0 {
        statuses.push((ButtonIcon::ChangeAdded, counts.added, "new"));
    }
    if counts.modified > 0 {
        statuses.push((ButtonIcon::ChangeModified, counts.modified, "modified"));
    }
    if counts.removed > 0 {
        statuses.push((ButtonIcon::ChangeRemoved, counts.removed, "removed"));
    }
    let statuses_width = |compact: bool, dots: bool| {
        statuses
            .iter()
            .enumerate()
            .map(|(index, (_, count, label))| {
                let value = if compact {
                    count.to_string()
                } else {
                    format!("{count} {label}")
                };
                (if index > 0 {
                    8.0 + if dots { text_width("•") + 8.0 } else { 0.0 }
                } else {
                    0.0
                }) + BUTTON_ICON_SIZE
                    + 4.0
                    + text_width(&value)
            })
            .sum::<f32>()
    };
    let fits = |title: &str, compact: bool, dots: bool| {
        let between = if statuses.is_empty() { 0.0 } else { 8.0 };
        text_width(title) + between + statuses_width(compact, dots) <= rect.width()
    };
    let mut compact = false;
    let mut dots = true;
    if !fits(&count_title, compact, dots) {
        compact = true;
    }
    if !fits(&count_title, compact, dots) {
        count_title = if filtered {
            format!("{} of {total_count} tags", files.len())
        } else {
            format!("{} tags", files.len())
        };
    }
    if !fits(&count_title, compact, dots) {
        dots = false;
    }
    if !fits(&count_title, compact, dots) {
        count_title = if filtered {
            format!("{}/{total_count}", files.len())
        } else {
            files.len().to_string()
        };
    }
    let mut x = rect.right() - statuses_width(compact, dots);
    let title_budget = if statuses.is_empty() {
        rect.width()
    } else {
        (x - rect.left() - 8.0).max(0.0)
    };
    let suffix = format!(" - {count_title}");
    let prefix_budget = title_budget - text_width(&suffix);
    let title = if prefix_budget >= text_width("…") {
        let prefix = truncate_end_to_width(ui, revision_title, &font, text_dark(), prefix_budget);
        format!("{prefix}{suffix}")
    } else {
        count_title
    };
    let title_clip = if statuses.is_empty() {
        rect
    } else {
        egui::Rect::from_min_max(
            rect.left_top(),
            egui::pos2((x - 8.0).max(rect.left()), rect.bottom()),
        )
    };
    ui.painter().with_clip_rect(title_clip).text(
        egui::pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &title,
        font.clone(),
        text_dark(),
    );
    let painter = ui.painter().with_clip_rect(rect);
    for (index, (icon, count, label)) in statuses.iter().enumerate() {
        if index > 0 {
            x += 8.0;
            if dots {
                painter.text(
                    egui::pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    "•",
                    font.clone(),
                    subtle_dark(),
                );
                x += text_width("•") + 8.0;
            }
        }
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(x + BUTTON_ICON_SIZE * 0.5, rect.center().y),
            Vec2::splat(BUTTON_ICON_SIZE),
        );
        if icon_rect.left() >= rect.left() && icon_rect.right() <= rect.right() {
            paint_button_icon_at(ui, *icon, icon_rect, text_dark());
        }
        x += BUTTON_ICON_SIZE + 4.0;
        let value = if compact {
            count.to_string()
        } else {
            format!("{count} {label}")
        };
        painter.text(
            egui::pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            &value,
            font.clone(),
            text_dark(),
        );
        x += text_width(&value);
    }
}

fn list_header(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::none()
        .inner_margin(egui::Margin {
            left: 10.0,
            right: 10.0,
            top: 8.0,
            bottom: 8.0,
        })
        .show(ui, content);
}

fn truncate_end_to_width(
    ui: &Ui,
    text: &str,
    font: &FontId,
    color: Color32,
    max_width: f32,
) -> String {
    super::tag_compare::truncate_end(text, max_width, |candidate| {
        text_width(ui, candidate, font, color)
    })
}

fn truncate_start_to_width(
    ui: &Ui,
    text: &str,
    font: &FontId,
    color: Color32,
    max_width: f32,
) -> String {
    super::tag_compare::truncate_start(text, max_width, |candidate| {
        text_width(ui, candidate, font, color)
    })
}

fn text_width(ui: &Ui, text: &str, font: &FontId, color: Color32) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color)
        .size()
        .x
}

fn commit_matches_filter(commit: &GitReviewCommit, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || commit.subject.to_lowercase().contains(&filter)
        || commit.author.to_lowercase().contains(&filter)
        || commit.hash.to_lowercase().contains(&filter)
        || commit.short_hash.to_lowercase().contains(&filter)
}

fn commit_row_tooltip(
    commit: &GitReviewCommit,
    shown_subject: &str,
    metadata: &str,
    shown_metadata: &str,
) -> Option<String> {
    match (shown_subject != commit.subject, shown_metadata != metadata) {
        (true, true) => Some(format!("{}\n{metadata}", commit.subject)),
        (true, false) => Some(commit.subject.clone()),
        (false, true) => Some(metadata.to_owned()),
        (false, false) => None,
    }
}

fn commit_row(ui: &mut Ui, commit: &GitReviewCommit, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::click());
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        editor_bg()
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    let border_y = rect.bottom() - 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), border_y),
            egui::pos2(rect.right(), border_y),
        ],
        Stroke::new(1.0_f32, grid_line()),
    );
    let subject_color = if selected {
        Color32::WHITE
    } else {
        text_dark()
    };
    let text_rect = rect.shrink2(egui::vec2(8.0, 0.0));
    let subject_font = bold_font(12.0);
    let shown_subject = truncate_end_to_width(
        ui,
        &commit.subject,
        &subject_font,
        subject_color,
        text_rect.width(),
    );
    ui.painter().with_clip_rect(text_rect).text(
        egui::pos2(rect.left() + 8.0, rect.top() + 9.0),
        Align2::LEFT_TOP,
        shown_subject.clone(),
        subject_font,
        subject_color,
    );
    let metadata = [
        commit.short_hash.as_str(),
        commit.author.as_str(),
        commit.date.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("  ·  ");
    let metadata_font = FontId::proportional(11.0);
    let metadata_color = if selected {
        Color32::WHITE.gamma_multiply(0.78)
    } else {
        subtle_dark()
    };
    let shown_metadata = truncate_end_to_width(
        ui,
        &metadata,
        &metadata_font,
        metadata_color,
        text_rect.width(),
    );
    ui.painter().with_clip_rect(text_rect).text(
        egui::pos2(rect.left() + 8.0, rect.top() + 31.0),
        Align2::LEFT_TOP,
        shown_metadata.clone(),
        metadata_font,
        metadata_color,
    );
    if let Some(tooltip) = commit_row_tooltip(commit, &shown_subject, &metadata, &shown_metadata) {
        response.on_hover_text(tooltip)
    } else {
        response
    }
}

fn tag_change_row(ui: &mut Ui, file: &GitReviewFile, selected: bool) -> egui::Response {
    let display_path = native_git_display_path(&file.path);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    let border_y = rect.bottom() - 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), border_y),
            egui::pos2(rect.right(), border_y),
        ],
        Stroke::new(1.0_f32, grid_line()),
    );
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    paint_tag_icon_at(ui, Some(file.group_tag), icon_rect);
    let (prefix, name) = display_path
        .rfind(['/', '\\'])
        .map_or(("", display_path.as_str()), |split| {
            display_path.split_at(split + 1)
        });
    let font = TextStyle::Body.resolve(ui.style());
    let change_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 12.0, rect.center().y),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    let text_pos = egui::pos2(rect.left() + 28.0, rect.center().y);
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(text_pos.x, rect.top()),
        egui::pos2(change_rect.left() - 6.0, rect.bottom()),
    );
    let available = text_rect.width().max(0.0);
    let name_width = ui
        .painter()
        .layout_no_wrap(name.to_owned(), font.clone(), text_dark())
        .size()
        .x;
    let (shown_prefix, shown_name) = if name_width >= available {
        (
            String::new(),
            truncate_start_to_width(ui, name, &font, text_dark(), available),
        )
    } else {
        (
            truncate_start_to_width(
                ui,
                prefix,
                &font,
                text_dark().gamma_multiply(0.5),
                available - name_width,
            ),
            name.to_owned(),
        )
    };
    let truncated = shown_prefix != prefix || shown_name != name;
    let prefix_width = ui
        .painter()
        .layout_no_wrap(
            shown_prefix.clone(),
            font.clone(),
            text_dark().gamma_multiply(0.5),
        )
        .size()
        .x;
    let painter = ui.painter().with_clip_rect(text_rect);
    painter.text(
        text_pos,
        Align2::LEFT_CENTER,
        shown_prefix,
        font.clone(),
        text_dark().gamma_multiply(0.5),
    );
    painter.text(
        text_pos + egui::vec2(prefix_width, 0.0),
        Align2::LEFT_CENTER,
        shown_name,
        font,
        text_dark(),
    );
    paint_button_icon_at(ui, change_icon(&file.status), change_rect, text_dark());
    if truncated {
        response.on_hover_text(display_path)
    } else {
        response
    }
}

impl Baboon {
    pub(super) fn draw_git_review(&mut self, ui: &mut Ui, kit_index: usize) {
        // Borrowed for the draw, which reads the review and writes only locals:
        // the commit list, the change list and a diff of up to 5,000 rows used
        // to be copied out every frame.
        let state = &self.kits[kit_index].git_review;
        let branch = state.branch.clone();
        let repo = state.repo_root.clone();
        // Looking for GitHub Desktop stats the disk (on macOS, every folder on
        // PATH), so it is asked once a second rather than every frame.
        let github_desktop =
            recheck_cached(ui.ctx(), "github_desktop_launcher", github_desktop_launcher);
        let commits = &state.commits;
        let files = &state.files;
        let selection = state.selection.clone();
        let revision_title = selected_revision_title(&selection, commits);
        let selected_path = state.selected_path.clone();
        let results = &state.results;
        let error = state.error.clone();
        let loading = state.loading;
        let local_count = state.local_files.len();
        let mut commit_filter = state.commit_filter.clone();
        let mut filter_text = state.filter.clone();
        let mut filters = state.filters;
        let mut swapped = state.swapped;
        let mut action = None;

        Frame::none()
            .show(ui, |ui| {
                // Only the page-level stack is flush. The header and each pane
                // keep their own inner spacing, but no gap is inserted between
                // the full-width divider and the three-pane table.
                let inner_spacing_y = ui.spacing().item_spacing.y;
                ui.spacing_mut().item_spacing.y = 0.0;
                Frame::none()
                    .inner_margin(egui::Margin {
                        left: 10.0,
                        right: 10.0,
                        top: 8.0,
                        bottom: 0.0,
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = inner_spacing_y;
                        ui.add_space(10.0);
                        let refresh_inline = ui.available_width()
                            >= if github_desktop.is_some() {
                                900.0
                            } else {
                                760.0
                            };
                        ui.horizontal(|ui| {
                            ui.add(button_icon_image(
                                ui,
                                ButtonIcon::Git,
                                text_dark(),
                                PANE_HEADER_ICON_SIZE,
                            ));
                            ui.vertical(|ui| {
                                ui.heading(
                                    RichText::new(GIT_REVIEW_TITLE).color(text_dark()).strong(),
                                );
                                let location = repo
                                    .as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_else(|| "No Git repository found".to_owned());
                                ui.label(
                                    RichText::new(if branch.is_empty() {
                                        location
                                    } else {
                                        format!("{branch}  ·  {location}")
                                    })
                                    .small()
                                    .color(subtle_dark()),
                                );
                            });
                            if refresh_inline {
                                git_review_header_actions(
                                    ui,
                                    repo.is_some(),
                                    github_desktop.is_some(),
                                    &mut action,
                                );
                            }
                        });
                        if !refresh_inline {
                            ui.add_space(8.0);
                            git_review_header_actions(
                                ui,
                                repo.is_some(),
                                github_desktop.is_some(),
                                &mut action,
                            );
                        }
                        ui.add_space(20.0);
                    });
                let (divider, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 1.0),
                    Sense::hover(),
                );
                ui.painter().line_segment(
                    [divider.left_top(), divider.right_top()],
                    Stroke::new(1.0_f32, grid_line()),
                );

                if loading {
                    Frame::none()
                        .inner_margin(egui::Margin {
                            left: 10.0,
                            right: 10.0,
                            top: 0.0,
                            bottom: 8.0,
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Reading Git…").color(subtle_dark()));
                            });
                        });
                }
                if let Some(error) = error.as_ref() {
                    Frame::none()
                        .inner_margin(egui::Margin {
                            left: 10.0,
                            right: 10.0,
                            top: 0.0,
                            bottom: 8.0,
                        })
                        .show(ui, |ui| {
                            ui.colored_label(Color32::from_rgb(225, 105, 105), error);
                        });
                }

                let width = ui.available_width();
                let height = ui.available_height().max(120.0);
                egui_extras::TableBuilder::new(ui)
                    .id_salt(("git_review_columns", kit_index))
                    .resizable(true)
                    .vscroll(false)
                    .column(egui_extras::Column::initial(width * 0.25).at_least(180.0))
                    .column(egui_extras::Column::initial(width * 0.32).at_least(220.0))
                    .column(egui_extras::Column::remainder().at_least(280.0))
                    .body(|mut body| {
                        body.row(height, |mut row| {
                            row.col(|ui| {
                                Frame::none().fill(editor_bg()).show(ui, |ui| {
                                    list_header(ui, |ui| {
                                            pane_title(ui, "Changes & Commit History");
                                            ui.add_space(4.0);
                                            browser_search_field(
                                                ui,
                                                &mut commit_filter,
                                                "search commits",
                                            );
                                    });
                                    ui.separator();
                                    egui::ScrollArea::vertical()
                                        .id_salt(("git_review_revisions", kit_index))
                                        .show(ui, |ui| {
                                        let local_selected =
                                            selection == GitReviewSelection::Local;
                                        let local = GitReviewCommit {
                                            hash: String::new(),
                                            short_hash: format!("{local_count} changed tag(s)"),
                                            author: String::new(),
                                            date: String::new(),
                                            subject: "Local Changes".to_owned(),
                                        };
                                        if commit_row(ui, &local, local_selected).clicked() {
                                            action = Some(GitReviewAction::SelectRevision(
                                                GitReviewSelection::Local,
                                            ));
                                        }
                                        for commit in commits
                                            .iter()
                                            .filter(|commit| commit_matches_filter(commit, &commit_filter))
                                        {
                                            let selected = matches!(&selection, GitReviewSelection::Commit(hash) if hash == &commit.hash);
                                            if commit_row(ui, commit, selected).clicked() {
                                                action = Some(GitReviewAction::SelectRevision(
                                                    GitReviewSelection::Commit(
                                                        commit.hash.clone(),
                                                    ),
                                                ));
                                            }
                                        }
                                        });
                                });
                            });
                            row.col(|ui| {
                                let mut visible_files = Vec::new();
                                list_header(ui, |ui| {
                                        let (header_rect, _) = ui.allocate_exact_size(
                                            Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                                            Sense::hover(),
                                        );
                                        ui.add_space(4.0);
                                        browser_search_field(ui, &mut filter_text, "search tags");
                                        let filter = filter_text.trim().to_lowercase();
                                        visible_files = files
                                            .iter()
                                            .filter(|file| file.path.to_lowercase().contains(&filter))
                                            .collect();
                                        change_list_header(
                                            ui,
                                            header_rect,
                                            &visible_files,
                                            files.len(),
                                            !filter.is_empty(),
                                            &revision_title,
                                        );
                                });
                                ui.separator();
                                egui::ScrollArea::vertical()
                                    .id_salt(("git_review_files", kit_index))
                                    .show(ui, |ui| {
                                        if visible_files.is_empty() {
                                            ui.add_space(10.0);
                                            ui.label(
                                                RichText::new(if filter_text.trim().is_empty() {
                                                    "No changed tags in this revision."
                                                } else {
                                                    "No tags match this search."
                                                })
                                                .color(subtle_dark()),
                                            );
                                        }
                                        for file in visible_files {
                                            let selected = selected_path.as_deref()
                                                == Some(file.path.as_str());
                                            let response = tag_change_row(ui, file, selected);
                                            if response.double_clicked() {
                                                action = Some(GitReviewAction::OpenFile(
                                                    file.path.clone(),
                                                ));
                                            } else if response.clicked() {
                                                action = Some(GitReviewAction::SelectFile(
                                                    file.path.clone(),
                                                ));
                                            }
                                        }
                                    });
                            });
                            row.col(|ui| {
                                Frame::none()
                                    .inner_margin(egui::Margin {
                                        left: 10.0,
                                        right: 10.0,
                                        top: 8.0,
                                        bottom: 8.0,
                                    })
                                    .show(ui, |ui| {
                                        compare_pane_title(ui, selected_path.as_deref());
                                    });
                                ui.separator();
                                if let Some(results) = results.as_ref() {
                                    let (before, after) = match &selection {
                                        GitReviewSelection::Local => ("HEAD", "Working copy"),
                                        GitReviewSelection::Commit(_) => ("Before", "After"),
                                    };
                                    super::tag_compare::draw_tag_diff_list(
                                        ui,
                                        results,
                                        &mut filters,
                                        &mut swapped,
                                        before,
                                        after,
                                        "git_review_tag_diff_table",
                                    );
                                } else {
                                    ui.label(
                                        RichText::new(
                                            "Choose a tag to inspect its field changes.",
                                        )
                                        .color(subtle_dark()),
                                    );
                                }
                            });
                        });
                    });
            });

        let state = &mut self.kits[kit_index].git_review;
        state.commit_filter = commit_filter;
        state.filter = filter_text;
        state.filters = filters;
        state.swapped = swapped;

        match action {
            Some(GitReviewAction::OpenRepositoryFolder) => {
                if let Some(repo) = repo {
                    #[cfg(target_os = "windows")]
                    self.open_folder_in_explorer(repo, "Repository");
                    #[cfg(not(target_os = "windows"))]
                    {
                        if !repo.is_dir() {
                            self.status =
                                format!("Repository folder not found: {}", repo.display());
                        } else {
                            #[cfg(target_os = "macos")]
                            let opener = "open";
                            #[cfg(not(target_os = "macos"))]
                            let opener = "xdg-open";
                            self.status = match Command::new(opener).arg(&repo).spawn() {
                                Ok(_) => format!("Opened repository folder: {}", repo.display()),
                                Err(error) => format!("Could not open repository folder: {error}"),
                            };
                        }
                    }
                }
            }
            Some(GitReviewAction::OpenGitHubDesktop) => {
                if let (Some(launcher), Some(repo)) = (github_desktop.as_ref(), repo.as_ref()) {
                    self.status = match launcher.open(repo) {
                        Ok(()) => format!("Opening {} in GitHub Desktop", repo.display()),
                        Err(error) => format!("Could not open GitHub Desktop: {error}"),
                    };
                }
            }
            Some(GitReviewAction::Refresh) => {
                self.run_git_review_job(kit_index, GitReviewJob::Refresh, ui.ctx());
            }
            Some(GitReviewAction::SelectRevision(selection)) => {
                self.run_git_review_job(
                    kit_index,
                    GitReviewJob::SelectRevision(selection),
                    ui.ctx(),
                );
            }
            Some(GitReviewAction::SelectFile(path)) => {
                self.run_git_review_job(kit_index, GitReviewJob::SelectFile(path), ui.ctx());
            }
            Some(GitReviewAction::OpenFile(path)) => {
                self.open_git_review_file(kit_index, &path);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit() -> GitReviewCommit {
        GitReviewCommit {
            hash: "9d31cd291ebb8d3e".to_owned(),
            short_hash: "9d31cd2".to_owned(),
            author: "Paddy Tee".to_owned(),
            date: "2026-09-20".to_owned(),
            subject: "Add Git Review search".to_owned(),
        }
    }

    #[test]
    fn commit_filter_matches_title_author_and_hash_case_insensitively() {
        let commit = commit();
        assert!(commit_matches_filter(&commit, "git review"));
        assert!(commit_matches_filter(&commit, "PADDY"));
        assert!(commit_matches_filter(&commit, "9D31CD2"));
        assert!(!commit_matches_filter(&commit, "unrelated"));
    }

    #[test]
    fn empty_commit_filter_matches_every_commit() {
        assert!(commit_matches_filter(&commit(), "  "));
    }

    #[test]
    fn commit_tooltip_only_shows_truncated_text_without_full_hash() {
        let commit = commit();
        let metadata = "9d31cd2  ·  Paddy Tee  ·  2026-09-20";
        assert_eq!(
            commit_row_tooltip(&commit, &commit.subject, metadata, metadata),
            None
        );
        assert_eq!(
            commit_row_tooltip(&commit, "Add Git…", metadata, metadata),
            Some(commit.subject.clone())
        );
        assert_eq!(
            commit_row_tooltip(&commit, &commit.subject, metadata, "9d31cd2…"),
            Some(metadata.to_owned())
        );
        assert_eq!(
            commit_row_tooltip(&commit, "Add Git…", metadata, "9d31cd2…"),
            Some(format!("{}\n{metadata}", commit.subject))
        );
        assert!(
            !commit_row_tooltip(&commit, "Add Git…", metadata, "9d31cd2…")
                .unwrap()
                .contains(&commit.hash)
        );
    }

    #[test]
    fn changed_tags_header_uses_selected_revision_title() {
        let commit = commit();
        assert_eq!(
            selected_revision_title(&GitReviewSelection::Local, &[commit.clone()]),
            "Local Changes"
        );
        assert_eq!(
            selected_revision_title(&GitReviewSelection::Commit(commit.hash.clone()), &[commit],),
            "Add Git Review search"
        );
    }

    #[test]
    fn change_summary_groups_git_statuses() {
        let files = ['A', '?', 'M', 'R', 'D']
            .into_iter()
            .map(|status| GitReviewFile {
                status: status.to_string(),
                path: format!("{status}.tag"),
                group_tag: 0,
            })
            .collect::<Vec<_>>();
        let counts = change_counts(&files);
        assert_eq!(counts.added, 2);
        assert_eq!(counts.modified, 2);
        assert_eq!(counts.removed, 1);
    }
}
