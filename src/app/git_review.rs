//! Read-only Git repository discovery and semantic tag comparison state.

use super::*;

pub(in crate::app) const GIT_REVIEW_KEY: &str = "tool:git_review";
pub(in crate::app) const GIT_REVIEW_TITLE: &str = "Git Review";

/// Git always reports repository-relative paths with `/`. Build the working
/// tree path one component at a time so the filesystem receives native path
/// components without changing the string later passed back to Git.
fn git_worktree_path(repo: &Path, git_path: &str) -> PathBuf {
    git_path
        .split('/')
        .filter(|component| !component.is_empty())
        .fold(repo.to_path_buf(), |mut path, component| {
            path.push(component);
            path
        })
}

pub(in crate::app) fn native_git_display_path(git_path: &str) -> String {
    #[cfg(windows)]
    {
        git_path.replace('/', "\\")
    }
    #[cfg(not(windows))]
    {
        git_path.to_owned()
    }
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    left == right
        || matches!(
            (fs::canonicalize(left), fs::canonicalize(right)),
            (Ok(left), Ok(right)) if left == right
        )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) enum GitReviewSelection {
    Local,
    Commit(String),
}

impl Default for GitReviewSelection {
    fn default() -> Self {
        Self::Local
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct GitReviewCommit {
    pub(in crate::app) hash: String,
    pub(in crate::app) short_hash: String,
    pub(in crate::app) author: String,
    pub(in crate::app) date: String,
    pub(in crate::app) subject: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct GitReviewFile {
    pub(in crate::app) status: String,
    pub(in crate::app) path: String,
    pub(in crate::app) group_tag: u32,
}

#[derive(Default)]
pub(in crate::app) struct GitReviewState {
    pub(in crate::app) repo_root: Option<PathBuf>,
    pub(in crate::app) branch: String,
    pub(in crate::app) commits: Vec<GitReviewCommit>,
    pub(in crate::app) local_files: Vec<GitReviewFile>,
    pub(in crate::app) files: Vec<GitReviewFile>,
    pub(in crate::app) selection: GitReviewSelection,
    pub(in crate::app) selected_path: Option<String>,
    pub(in crate::app) results: Option<TagDiffResults>,
    /// A double-clicked working tag waiting for the tiled layout to be restored.
    pub(in crate::app) pending_open: Option<String>,
    pub(in crate::app) filters: TagDiffFilters,
    pub(in crate::app) swapped: bool,
    pub(in crate::app) commit_filter: String,
    pub(in crate::app) filter: String,
    pub(in crate::app) error: Option<String>,
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("Could not run Git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if message.is_empty() {
            format!("Git exited with {}", output.status)
        } else {
            message
        })
    }
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    git_output(root, args).map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
}

fn git_text_untrimmed(root: &Path, args: &[&str]) -> Result<String, String> {
    git_output(root, args).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn tag_group_for_path(path: &str) -> Option<u32> {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(extension_to_group_tag)
}

fn parse_commits(text: &str) -> Vec<GitReviewCommit> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(5, '\t');
            Some(GitReviewCommit {
                hash: fields.next()?.to_owned(),
                short_hash: fields.next()?.to_owned(),
                date: fields.next()?.to_owned(),
                author: fields.next()?.to_owned(),
                subject: fields.next()?.to_owned(),
            })
        })
        .collect()
}

fn parse_name_status(text: &str) -> Vec<GitReviewFile> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let status = fields.next()?.trim();
            let first = fields.next()?;
            let path = fields.next().unwrap_or(first);
            Some(GitReviewFile {
                status: status.chars().next().unwrap_or('?').to_string(),
                path: path.to_owned(),
                group_tag: tag_group_for_path(path)?,
            })
        })
        .collect()
}

fn parse_local_status(text: &str) -> Vec<GitReviewFile> {
    text.lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let status = line[..2].trim();
            let raw_path = line[3..].trim();
            let path = raw_path
                .rsplit_once(" -> ")
                .map_or(raw_path, |(_, to)| to)
                .trim_matches('"');
            Some(GitReviewFile {
                status: if status == "??" { "?" } else { status }.to_owned(),
                path: path.to_owned(),
                group_tag: tag_group_for_path(path)?,
            })
        })
        .collect()
}

fn retain_source_tags(files: &mut Vec<GitReviewFile>, repo: &Path, source_root: &Path) {
    files.retain(|file| git_worktree_path(repo, &file.path).starts_with(source_root));
}

fn sort_files_by_full_path(files: &mut [GitReviewFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

fn can_restore_review_selection(
    previous_repo: Option<&Path>,
    previous_selection: &GitReviewSelection,
    repo: &Path,
    commits: &[GitReviewCommit],
) -> bool {
    previous_repo == Some(repo)
        && match previous_selection {
            GitReviewSelection::Local => true,
            GitReviewSelection::Commit(hash) => commits.iter().any(|commit| &commit.hash == hash),
        }
}

impl Baboon {
    pub(super) fn open_git_review(&mut self) {
        let kit = self.active;
        let Some(root) = self.kits[kit]
            .source
            .as_ref()
            .and_then(|source| match &source.source {
                TagSource::LooseFolder { root, .. } => Some(root.clone()),
                _ => None,
            })
        else {
            self.status = "Git Review is available for folder-based editing kits".to_owned();
            return;
        };
        self.kits[kit].open_tag_pane(GIT_REVIEW_KEY);
        self.refresh_git_review(kit, &root);
    }

    pub(in crate::app) fn refresh_git_review(&mut self, kit: usize, source_root: &Path) {
        let previous_repo = self.kits[kit].git_review.repo_root.clone();
        let previous_selection = self.kits[kit].git_review.selection.clone();
        let previous_path = self.kits[kit].git_review.selected_path.clone();
        let result = (|| {
            // Git for Windows prints the worktree root with `/` separators.
            // Keep Git's relative file paths untouched, but store the root in
            // native form for shell handoffs such as File Explorer.
            let repo = PathBuf::from(native_git_display_path(&git_text(
                source_root,
                &["rev-parse", "--show-toplevel"],
            )?));
            let branch = git_text(&repo, &["branch", "--show-current"])?;
            let log = git_text(
                &repo,
                &["log", "-n", "100", "--format=%H%x09%h%x09%cs%x09%an%x09%s"],
            )?;
            let mut files = parse_local_status(&git_text_untrimmed(
                &repo,
                &["status", "--short", "--untracked-files=all"],
            )?);
            retain_source_tags(&mut files, &repo, source_root);
            sort_files_by_full_path(&mut files);
            Ok::<_, String>((repo, branch, parse_commits(&log), files))
        })();

        match result {
            Ok((repo, branch, commits, files)) => {
                let restore_previous = can_restore_review_selection(
                    previous_repo.as_deref(),
                    &previous_selection,
                    &repo,
                    &commits,
                );
                let selection = if restore_previous {
                    previous_selection
                } else {
                    GitReviewSelection::Local
                };
                let state = &mut self.kits[kit].git_review;
                state.repo_root = Some(repo);
                state.branch = branch;
                state.commits = commits;
                state.local_files = files.clone();
                state.files = files;
                state.selection = GitReviewSelection::Local;
                state.selected_path = None;
                state.results = None;
                state.error = None;
                if selection != GitReviewSelection::Local {
                    self.select_git_review_revision(kit, selection);
                    if self.kits[kit].git_review.error.is_some() {
                        let state = &mut self.kits[kit].git_review;
                        state.selection = GitReviewSelection::Local;
                        state.files = state.local_files.clone();
                        return;
                    }
                }
                if restore_previous
                    && let Some(path) = previous_path
                    && self.kits[kit].git_review.error.is_none()
                    && self.kits[kit]
                        .git_review
                        .files
                        .iter()
                        .any(|file| file.path == path)
                {
                    self.select_git_review_file(kit, path);
                }
            }
            Err(error) => {
                let state = &mut self.kits[kit].git_review;
                state.repo_root = None;
                state.error = Some(error);
            }
        }
    }

    pub(in crate::app) fn select_git_review_revision(
        &mut self,
        kit: usize,
        selection: GitReviewSelection,
    ) {
        let Some(repo) = self.kits[kit].git_review.repo_root.clone() else {
            return;
        };
        let mut files = match &selection {
            GitReviewSelection::Local => {
                git_text_untrimmed(&repo, &["status", "--short", "--untracked-files=all"])
                    .map(|text| parse_local_status(&text))
            }
            GitReviewSelection::Commit(hash) => git_text(
                &repo,
                &[
                    "diff-tree",
                    "--root",
                    "--no-commit-id",
                    "--name-status",
                    "-r",
                    hash,
                ],
            )
            .map(|text| parse_name_status(&text)),
        };
        if let Ok(files) = &mut files
            && let Some(source_root) =
                self.kits[kit]
                    .source
                    .as_ref()
                    .and_then(|source| match &source.source {
                        TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                        _ => None,
                    })
        {
            retain_source_tags(files, &repo, source_root);
        }
        let local_selected = selection == GitReviewSelection::Local;
        if local_selected && let Ok(files) = &mut files {
            sort_files_by_full_path(files);
        }
        let state = &mut self.kits[kit].git_review;
        state.selection = selection;
        state.selected_path = None;
        state.results = None;
        match files {
            Ok(files) => {
                if local_selected {
                    state.local_files = files.clone();
                }
                state.files = files;
                state.error = None;
            }
            Err(error) => state.error = Some(error),
        }
    }

    pub(in crate::app) fn select_git_review_file(&mut self, kit: usize, path: String) {
        let result = self.git_review_comparison(kit, &path);
        let state = &mut self.kits[kit].git_review;
        state.selected_path = Some(path);
        match result {
            Ok(results) => {
                state.results = Some(results);
                state.error = None;
            }
            Err(error) => {
                state.results = None;
                state.error = Some(error);
            }
        }
    }

    pub(in crate::app) fn open_git_review_file(&mut self, kit: usize, path: &str) {
        let Some(repo) = self.kits[kit].git_review.repo_root.as_ref() else {
            return;
        };
        let absolute = git_worktree_path(repo, path);
        if !absolute.is_file() {
            self.status = format!("Cannot open deleted tag {}", native_git_display_path(path));
            return;
        }

        let existing_key = self.kits[kit].source.as_ref().and_then(|source| {
            source
                .entries
                .iter()
                .chain(source.all_entries.iter())
                .find_map(|entry| match &entry.location {
                    TagEntryLocation::LooseFile(existing)
                        if paths_refer_to_same_file(existing, &absolute) =>
                    {
                        Some(entry.key.clone())
                    }
                    _ => None,
                })
        });
        let key = if let Some(key) = existing_key {
            key
        } else {
            let new_entry = self.kits[kit].source.as_ref().and_then(|source| {
                let TagSource::LooseFolder { root, .. } = &source.source else {
                    return None;
                };
                let absolute = fs::canonicalize(&absolute).unwrap_or_else(|_| absolute.clone());
                loose_file_entry(root, &absolute, &source.names)
                    .ok()
                    .flatten()
            });
            let Some(entry) = new_entry else {
                self.status = format!(
                    "Cannot find tag {} in the loaded editing kit",
                    native_git_display_path(path)
                );
                return;
            };
            let key = entry.key.clone();
            let folder_seeds = self.kits[kit].folder_seeds();
            if let Some(source) = self.kits[kit].source.as_mut() {
                source.upsert_entry(entry, &folder_seeds);
            }
            self.kits[kit].generation = self.kits[kit].generation.wrapping_add(1);
            key
        };
        self.kits[kit].git_review.pending_open = Some(key);
    }

    fn git_review_comparison(&self, kit: usize, path: &str) -> Result<TagDiffResults, String> {
        let state = &self.kits[kit].git_review;
        let repo = state
            .repo_root
            .as_ref()
            .ok_or_else(|| "No Git repository is loaded.".to_owned())?;
        let file = state
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(|| "The selected tag is no longer in this change set.".to_owned())?;
        let source = self.kits[kit]
            .source
            .as_ref()
            .ok_or_else(|| "The editing kit is no longer loaded.".to_owned())?;
        let definitions_root = match &source.source {
            TagSource::LooseFolder {
                definitions_root, ..
            } => definitions_root.as_path(),
            _ => return Err("Git Review requires a folder-based editing kit.".to_owned()),
        };
        let game = source.game.as_deref();
        let load_revision = |revision: &str| -> Result<Option<TagFile>, String> {
            let object = format!("{revision}:{path}");
            let exists = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["cat-file", "-e", &object])
                .output()
                .map_err(|error| format!("Could not run Git: {error}"))?;
            if !exists.status.success() {
                return Ok(None);
            }
            let bytes = git_output(repo, &["show", &object])?;
            crate::source::read_tag_from_bytes(&bytes, game, Some(definitions_root), file.group_tag)
                .map(Some)
                .map_err(|error| format!("Could not parse {path} at {revision}: {error}"))
        };
        let load_working = || -> Result<Option<TagFile>, String> {
            let absolute = git_worktree_path(repo, path);
            if !absolute.is_file() {
                return Ok(None);
            }
            crate::source::read_tag_at_path(&absolute, game, Some(definitions_root), file.group_tag)
                .map(Some)
                .map_err(|error| format!("Could not parse working tag {path}: {error}"))
        };
        let (before, after) = match &state.selection {
            GitReviewSelection::Local => (load_revision("HEAD")?, load_working()?),
            GitReviewSelection::Commit(hash) => {
                let parent = git_text(repo, &["rev-list", "--parents", "-n", "1", hash])?
                    .split_whitespace()
                    .nth(1)
                    .map(str::to_owned);
                let before = parent.as_deref().map(&load_revision).transpose()?.flatten();
                (before, load_revision(hash)?)
            }
        };
        Ok(
            crate::app::ui::tag_compare::comparison_results_with_missing(
                before.as_ref(),
                after.as_ref(),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parsers_hide_non_tags_and_keep_tag_types() {
        let files =
            parse_local_status(" M tags/a.weapon\nR  old/path -> tags/b.weapon\n?? notes.txt\n");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, "M");
        assert_eq!(files[0].path, "tags/a.weapon");
        assert_eq!(files[0].group_tag, u32::from_be_bytes(*b"weap"));
        assert_eq!(files[1].path, "tags/b.weapon");
    }

    #[test]
    fn parses_commit_name_status() {
        let files =
            parse_name_status("M\ttags/a.weapon\nR100\told.weapon\tnew.weapon\nD\tREADME.md\n");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, "M");
        assert_eq!(files[1].path, "new.weapon");
    }

    #[test]
    fn git_paths_become_native_worktree_paths_without_changing_git_strings() {
        let repo = Path::new("repo-root");
        let path = git_worktree_path(repo, "tags/objects/rifle.weapon");
        assert_eq!(path, repo.join("tags").join("objects").join("rifle.weapon"));
        #[cfg(windows)]
        assert_eq!(
            native_git_display_path("tags/objects/rifle.weapon"),
            r"tags\objects\rifle.weapon"
        );
        #[cfg(not(windows))]
        assert_eq!(
            native_git_display_path("tags/objects/rifle.weapon"),
            "tags/objects/rifle.weapon"
        );
    }

    #[test]
    fn git_repository_root_uses_native_separators_for_shell_handoffs() {
        let root = PathBuf::from(native_git_display_path("C:/Program Files/H2CE"));
        #[cfg(windows)]
        assert_eq!(root.to_string_lossy(), r"C:\Program Files\H2CE");
        #[cfg(not(windows))]
        assert_eq!(root.to_string_lossy(), "C:/Program Files/H2CE");
    }

    #[test]
    fn local_changes_sort_by_full_path_not_status() {
        let mut files = parse_local_status(
            "?? tags/z.weapon\n M tags/b.weapon\n?? tags/a.weapon\n D tags/c.weapon\n",
        );
        sort_files_by_full_path(&mut files);
        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            [
                "tags/a.weapon",
                "tags/b.weapon",
                "tags/c.weapon",
                "tags/z.weapon"
            ]
        );
    }

    #[test]
    fn refresh_restores_only_revisions_still_in_the_same_repository() {
        let repo = Path::new("repo");
        let commit = GitReviewCommit {
            hash: "abc123".to_owned(),
            short_hash: "abc123".to_owned(),
            author: String::new(),
            date: String::new(),
            subject: String::new(),
        };
        assert!(can_restore_review_selection(
            Some(repo),
            &GitReviewSelection::Local,
            repo,
            &[],
        ));
        assert!(can_restore_review_selection(
            Some(repo),
            &GitReviewSelection::Commit(commit.hash.clone()),
            repo,
            &[commit],
        ));
        assert!(!can_restore_review_selection(
            Some(repo),
            &GitReviewSelection::Commit("missing".to_owned()),
            repo,
            &[],
        ));
        assert!(!can_restore_review_selection(
            Some(Path::new("other")),
            &GitReviewSelection::Local,
            repo,
            &[],
        ));
    }
}
