//! How a search's result tree opens: every folder (or group) starts closed,
//! with how many matches it holds, and opening one keeps opening while a level
//! holds a single item, so a lone path to the matches takes one click. Folders
//! the user opens or closes stay that way while the search is refined, and are
//! kept apart from the tree's own open state, which clearing the search shows
//! again as it was.
use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// A result node's key: its path from the tags root, lowercased, as tag paths
/// compare. A group node's path is its label.
pub(in crate::app) fn search_node_key(path: &Path) -> String {
    tag_path::relative_tag_path(path).to_ascii_lowercase()
}

/// Matches inside each node of a result tree, at every depth: its tags, and
/// the folders beneath it that `named` says matched themselves.
pub(super) fn match_counts(
    tree: &TagTree,
    named: &impl Fn(&TagTreeNode) -> bool,
) -> HashMap<String, usize> {
    fn count(
        node: &TagTreeNode,
        named: &impl Fn(&TagTreeNode) -> bool,
        counts: &mut HashMap<String, usize>,
    ) -> usize {
        let mut total = node.entries.len();
        for child in &node.children {
            total += count(child, named, counts) + usize::from(named(child));
        }
        counts.insert(search_node_key(&node.rel_path), total);
        total
    }
    let mut counts = HashMap::new();
    for child in &tree.children {
        count(child, named, &mut counts);
    }
    counts
}

/// Whether `key` is `root` or beneath it; an empty root holds everything.
fn is_beneath(key: &str, root: &str) -> bool {
    crate::app::kits::loading::scope_contains(root, key)
}

/// One search's open folders, for one tree. Reset when the search is cleared.
#[derive(Default)]
pub(in crate::app) struct SearchFolders {
    user_open: HashSet<String>,
    user_closed: HashSet<String>,
    /// What draws open: worked out from the results and the user's clicks.
    open: HashSet<String>,
    counts: Arc<HashMap<String, usize>>,
    /// The results `open` was worked out for, by [`FilterCache::serial`].
    built_for: Option<u64>,
    groups: bool,
    /// Moves whenever `open` changes, so cached folder heights are dropped.
    revision: u64,
    /// Opened or closed while drawing, applied at the next [`Self::sync`]:
    /// a node's key, whether it opens, and whether everything inside follows.
    toggles: RefCell<Vec<(String, bool, bool)>>,
}

impl SearchFolders {
    /// Forget the search: the next one starts with every folder closed.
    pub(in crate::app) fn clear(&mut self) {
        if self.built_for.is_some() || !self.user_open.is_empty() || !self.user_closed.is_empty() {
            *self = Self { revision: self.revision + 1, ..Self::default() };
        }
    }

    /// Bring the open folders up to date with the latest results and the
    /// clicks drawn since; `open_all` is Collapse All (Some(false)).
    pub(in crate::app) fn sync(&mut self, cache: &FilterCache, groups: bool, open_all: Option<bool>) {
        let toggles = std::mem::take(&mut *self.toggles.borrow_mut());
        let stale = self.built_for != Some(cache.serial) || self.groups != groups;
        if !stale && toggles.is_empty() && open_all.is_none() {
            return;
        }
        if self.groups != groups {
            self.user_open.clear();
            self.user_closed.clear();
        }
        self.groups = groups;
        for (key, open, everything) in toggles {
            if everything {
                self.set_all(&cache.tree, &key, open);
            } else {
                self.set(key, open);
            }
        }
        if let Some(open) = open_all {
            self.set_all(&cache.tree, "", open);
        }
        self.counts = Arc::clone(&cache.counts);
        self.built_for = Some(cache.serial);
        self.open = self.work_out(&cache.tree);
        self.revision += 1;
    }

    fn set(&mut self, key: String, open: bool) {
        if open {
            self.user_closed.remove(&key);
            self.user_open.insert(key);
        } else {
            self.user_open.remove(&key);
            self.user_closed.insert(key);
        }
    }

    fn set_all(&mut self, tree: &TagTree, root: &str, open: bool) {
        fn visit(node: &TagTreeNode, keys: &mut Vec<String>) {
            keys.push(search_node_key(&node.rel_path));
            for child in &node.children {
                visit(child, keys);
            }
        }
        let mut keys = Vec::new();
        for child in &tree.children {
            visit(child, &mut keys);
        }
        for key in keys.into_iter().filter(|key| is_beneath(key, root)) {
            self.set(key, open);
        }
    }

    fn work_out(&self, tree: &TagTree) -> HashSet<String> {
        let mut open = HashSet::new();
        let lone = tree.children.len() == 1 && tree.entries.is_empty();
        for child in &tree.children {
            // A group holding a single match has nothing to choose between.
            let single = self.groups && self.count(&child.rel_path) == Some(1);
            self.visit(child, lone || single, &mut open);
        }
        open
    }

    fn visit(&self, node: &TagTreeNode, chained: bool, open: &mut HashSet<String>) {
        let key = search_node_key(&node.rel_path);
        let is_open = !self.user_closed.contains(&key) && (chained || self.user_open.contains(&key));
        // An open folder holding only one folder opens that one too.
        let lone = is_open && node.children.len() == 1 && node.entries.is_empty();
        if is_open {
            open.insert(key);
        }
        for child in &node.children {
            self.visit(child, lone, open);
        }
    }

    pub(in crate::app) fn is_open(&self, path: &Path) -> bool {
        self.open.contains(&search_node_key(path))
    }

    pub(in crate::app) fn count(&self, path: &Path) -> Option<usize> {
        self.counts.get(&search_node_key(path)).copied()
    }

    pub(in crate::app) fn revision(&self) -> u64 {
        self.revision
    }

    /// A click opened or closed `path`; with `everything`, all inside it too.
    pub(in crate::app) fn toggle(&self, path: &Path, open: bool, everything: bool) {
        self.toggles
            .borrow_mut()
            .push((search_node_key(path), open, everything));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str, entries: usize, children: Vec<TagTreeNode>) -> TagTreeNode {
        TagTreeNode {
            label: tag_path::name(path).to_owned(),
            rel_path: PathBuf::from(path.replace('\\', std::path::MAIN_SEPARATOR_STR)),
            children,
            entries: (0..entries).collect(),
            ..Default::default()
        }
    }

    fn cache(tree: TagTree, serial: u64) -> FilterCache {
        let mut cache = FilterCache::default();
        cache.counts = Arc::new(match_counts(&tree, &|_| false));
        cache.tree = tree;
        cache.serial = serial;
        cache
    }

    fn opened(folders: &SearchFolders) -> Vec<String> {
        let mut open: Vec<_> = folders.open.iter().cloned().collect();
        open.sort();
        open
    }

    /// objects\characters\brute (3 tags) and objects\characters\elite (1),
    /// plus scenarios\solo (2).
    fn results() -> TagTree {
        TagTree {
            children: vec![
                node(
                    "objects",
                    0,
                    vec![node(
                        "objects\\characters",
                        0,
                        vec![
                            node("objects\\characters\\brute", 3, Vec::new()),
                            node("objects\\characters\\elite", 1, Vec::new()),
                        ],
                    )],
                ),
                node("scenarios", 0, vec![node("scenarios\\solo", 2, Vec::new())]),
            ],
            entries: Vec::new(),
        }
    }

    #[test]
    fn folders_start_closed_with_their_counts() {
        let cache = cache(results(), 1);
        let mut folders = SearchFolders::default();
        folders.sync(&cache, false, None);
        assert!(opened(&folders).is_empty());
        assert_eq!(folders.count(Path::new("objects")), Some(4));
        assert_eq!(folders.count(Path::new("scenarios")), Some(2));
    }

    #[test]
    fn opening_a_folder_follows_a_lone_path_until_it_branches() {
        let cache = cache(results(), 1);
        let mut folders = SearchFolders::default();
        folders.sync(&cache, false, None);
        folders.toggle(Path::new("objects"), true, false);
        folders.sync(&cache, false, None);
        // characters holds two folders, so the chain stops there.
        assert_eq!(opened(&folders), ["objects", "objects\\characters"]);
        folders.toggle(Path::new("scenarios"), true, false);
        folders.sync(&cache, false, None);
        assert!(folders.is_open(Path::new("scenarios\\solo")));
    }

    #[test]
    fn a_lone_top_folder_opens_by_itself_and_clicks_outlast_new_results() {
        let tree = || TagTree {
            children: vec![node("objects", 0, vec![node("objects\\weapons", 2, Vec::new())])],
            entries: Vec::new(),
        };
        let mut folders = SearchFolders::default();
        folders.sync(&cache(tree(), 1), false, None);
        assert_eq!(opened(&folders), ["objects", "objects\\weapons"]);
        folders.toggle(Path::new("objects"), false, false);
        folders.sync(&cache(tree(), 2), false, None);
        assert!(!folders.is_open(Path::new("objects")));
    }

    #[test]
    fn expand_all_and_collapse_all_reach_every_level() {
        let cache = cache(results(), 1);
        let mut folders = SearchFolders::default();
        folders.sync(&cache, false, Some(true));
        assert_eq!(opened(&folders).len(), 6);
        folders.toggle(Path::new("objects"), false, true);
        folders.sync(&cache, false, None);
        assert_eq!(opened(&folders), ["scenarios", "scenarios\\solo"]);
        folders.clear();
        assert!(opened(&folders).is_empty());
    }

    #[test]
    fn a_group_with_one_match_opens() {
        let tree = TagTree {
            children: vec![node("biped", 1, Vec::new()), node("model", 3, Vec::new())],
            entries: Vec::new(),
        };
        let mut folders = SearchFolders::default();
        folders.sync(&cache(tree, 1), true, None);
        assert_eq!(opened(&folders), ["biped"]);
    }

    #[test]
    fn folders_that_matched_by_name_count_in_the_folders_above() {
        let tree = results();
        let counts = match_counts(&tree, &|node: &TagTreeNode| node.label == "brute");
        assert_eq!(counts["objects"], 5);
        assert_eq!(counts["objects\\characters\\brute"], 3);
    }
}
