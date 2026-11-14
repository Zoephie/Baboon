//! Filesystem planning and rollback for recursive folder merges. Planning never
//! writes; callers supply review decisions and recovery-copy destinations.
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Replace,
    Skip,
    KeepBoth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    pub directory: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub readonly: bool,
    digest: Vec<u8>,
}

pub(crate) fn fingerprint(path: &Path) -> Result<Fingerprint, String> {
    fingerprint_cancellable(path, &|| false)
}

pub(crate) fn fingerprint_cancellable(
    path: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<Fingerprint, String> {
    fingerprint_inner(path, true, cancelled)
}

/// Metadata is sufficient to notice ordinary edits outside the reviewed files.
/// Reviewed files and their referrers additionally receive a content digest.
pub(crate) fn metadata_fingerprint(path: &Path) -> Result<Fingerprint, String> {
    fingerprint_inner(path, false, &|| false)
}

fn fingerprint_inner(
    path: &Path,
    contents: bool,
    cancelled: &impl Fn() -> bool,
) -> Result<Fingerprint, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("Could not inspect {}: {e}", path.display()))?;
    if meta.file_type().is_symlink() || (!meta.is_dir() && !meta.is_file()) {
        return Err(format!(
            "Links and special files cannot be merged: {}",
            path.display()
        ));
    }
    let mut digest = Sha256::new();
    if contents && meta.is_file() {
        let mut file =
            fs::File::open(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        let mut buffer = [0; 65536];
        loop {
            if cancelled() {
                return Err("Folder operation cancelled; no files changed".into());
            }
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
        }
    }
    Ok(Fingerprint {
        directory: meta.is_dir(),
        size: meta.len(),
        modified: meta.modified().ok(),
        readonly: meta.permissions().readonly(),
        digest: digest.finalize().to_vec(),
    })
}

/// Preserve the original timestamps in recovery and rollback copies.
pub(crate) fn copy_backup(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "Recovery source is no longer a regular file: {}",
            source.display()
        ));
    }
    if fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!(
            "Cannot restore through a link: {}",
            target.display()
        ));
    }
    fs::copy(source, target)
        .map_err(|e| format!("Could not preserve {}: {e}", source.display()))?;
    let mut times = fs::FileTimes::new();
    if let Ok(time) = metadata.modified() {
        times = times.set_modified(time);
    }
    if let Ok(time) = metadata.accessed() {
        times = times.set_accessed(time);
    }
    fs::OpenOptions::new()
        .write(true)
        .open(target)
        .and_then(|file| file.set_times(times))
        .map_err(|e| e.to_string())
}

#[derive(Clone, Debug)]
pub(crate) struct Conflict {
    pub incoming: PathBuf,
    pub existing: PathBuf,
    pub incoming_info: Fingerprint,
    pub existing_info: Fingerprint,
}
impl Conflict {
    pub fn replace_allowed(&self) -> bool {
        !self.incoming_info.directory && !self.existing_info.directory
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Inventory {
    roots: Vec<(PathBuf,PathBuf)>,
    pub source_items: BTreeMap<PathBuf, Fingerprint>,
    destination_items: BTreeMap<PathBuf, Fingerprint>,
    children: BTreeMap<PathBuf, Vec<PathBuf>>,
}

fn tree(
    path: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<PathBuf, Fingerprint>, String> {
    let mut items = BTreeMap::new();
    let mut names = HashSet::new();
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(items);
    }
    for item in walkdir::WalkDir::new(path)
        .follow_links(false)
        .sort_by_file_name()
    {
        if cancelled() {
            return Err("Folder operation cancelled; no files changed".into());
        }
        let item = item.map_err(|e| e.to_string())?;
        if !names.insert(path_key(item.path())) {
            return Err(format!(
                "Ambiguous names differing only in case: {}",
                item.path().display()
            ));
        }
        items.insert(
            item.path().to_owned(),
            fingerprint_cancellable(item.path(), cancelled)?,
        );
    }
    Ok(items)
}

/// Tag paths are case insensitive even on a case-sensitive host. Ambiguous
/// siblings are rejected rather than arbitrarily overwriting one of them.
pub(crate) fn matching_path(path: &Path) -> Result<Option<PathBuf>, String> {
    let Some(parent) = path.parent() else {
        return Ok(None);
    };
    if !parent.exists() {
        return Ok(None);
    }
    let leaf = path
        .file_name()
        .ok_or("Path has no leaf")?
        .to_string_lossy();
    let matches = fs::read_dir(parent)
        .map_err(|e| e.to_string())?
        .map(|e| e.map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&leaf))
        .map(|e| e.path())
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(format!(
            "Ambiguous names differing only in case: {}",
            path.display()
        ));
    }
    Ok(matches.into_iter().next())
}

impl Inventory {
    #[cfg(test)]
    pub fn scan(
        source: &Path,
        destination: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::scan_many(&[(source.to_owned(),destination.to_owned())],cancelled)
    }
    pub fn scan_many(roots: &[(PathBuf,PathBuf)], cancelled: &impl Fn()->bool) -> Result<Self,String> {
        let mut source_items = BTreeMap::new();
        let mut destination_items = BTreeMap::new();
        for (source,destination) in roots { source_items.extend(tree(source,cancelled)?); destination_items.extend(tree(destination,cancelled)?); }
        let mut children: BTreeMap<PathBuf,Vec<PathBuf>> = BTreeMap::new();
        for item in source_items.keys() {
            children.entry(item.parent().ok_or("Invalid source path")?.to_owned()).or_default().push(item.clone());
        }
        Ok(Self { roots:roots.to_vec(), source_items, destination_items, children })
    }
    pub fn unchanged(&self, cancelled: &impl Fn()->bool) -> Result<bool,String> {
        let current=Self::scan_many(&self.roots,cancelled)?;
        Ok(self.source_items==current.source_items && self.destination_items==current.destination_items)
    }
    pub fn files(&self) -> usize {
        self.source_items.values().filter(|i| !i.directory).count()
    }
    pub fn conflicts(&self, cancelled: &impl Fn() -> bool) -> Result<usize, String> {
        let mut count = 0;
        self.plan(
            |conflict| {
                count += 1;
                Ok(if conflict.replace_allowed() {
                    Choice::Replace
                } else {
                    Choice::Skip
                })
            },
            cancelled,
        )?;
        Ok(count)
    }
    pub fn plan(
        &self,
        mut decide: impl FnMut(Conflict) -> Result<Choice, String>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Plan, String> {
        let mut plan = Plan {
            files: Vec::new(),
            directories: Vec::new(),
            source_directories: Vec::new(),
            new_directories: HashSet::new(),
        };
        let mut reserved = HashSet::new();
        for (source,destination) in &self.roots {
            self.plan_item(source,destination,&mut plan,&mut reserved,&mut decide,cancelled)?;
        }
        Ok(plan)
    }
    fn plan_item(
        &self,
        source: &Path,
        requested: &Path,
        plan: &mut Plan,
        reserved: &mut HashSet<String>,
        decide: &mut impl FnMut(Conflict) -> Result<Choice, String>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if cancelled() {
            return Err("Folder operation cancelled; no files changed".into());
        }
        let info = self
            .source_items
            .get(source)
            .ok_or("Source changed during review")?;
        let existing = matching_path(requested)?;
        let mut target = existing.clone().unwrap_or_else(|| requested.to_owned());
        let mut replace = false;
        // Reserve unsuffixed sibling names too, even if their source files have
        // not been visited yet. Keep Both must not steal an incoming filename.
        let occupied = || {
            let mut occupied = reserved.clone();
            occupied.extend(self.roots.iter().map(|(_,target)| path_key(target)));
            if let Some(siblings) = source.parent().and_then(|parent| self.children.get(parent)) {
                occupied.extend(
                    siblings
                        .iter()
                        .map(|p| path_key(&requested.with_file_name(p.file_name().unwrap()))),
                );
            }
            occupied
        };
        if let Some(existing) = existing {
            let other = fingerprint(&existing)?;
            if !info.directory || !other.directory {
                let conflict = Conflict {
                    incoming: source.to_owned(),
                    existing,
                    incoming_info: info.clone(),
                    existing_info: other,
                };
                match decide(conflict.clone())? {
                    Choice::Skip => return Ok(()),
                    Choice::Replace if conflict.replace_allowed() => replace = true,
                    Choice::Replace => {
                        return Err("A file/folder mismatch cannot be replaced".into());
                    }
                    Choice::KeepBoth => {
                        target = unused_name(requested, info.directory, &occupied())?
                    }
                }
            }
        } else if reserved.contains(&path_key(&target)) {
            target = unused_name(&target, info.directory, &occupied())?;
        }
        reserved.insert(path_key(&target));
        if info.directory {
            if !target.exists() {
                plan.new_directories.insert(target.clone());
            }
            plan.directories.push(target.clone());
            plan.source_directories.push(source.to_owned());
            if let Some(children) = self.children.get(source) {
                for child in children {
                    self.plan_item(
                        child,
                        &target.join(child.file_name().unwrap()),
                        plan,
                        reserved,
                        decide,
                        cancelled,
                    )?;
                }
            }
        } else {
            plan.files.push(PlannedFile {
                source: source.to_owned(),
                target,
                replace,
            });
        }
        Ok(())
    }
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

pub(crate) fn snapshot(roots: &[PathBuf], cancelled: &impl Fn()->bool) -> Result<BTreeMap<PathBuf,Fingerprint>,String> {
    let mut result=BTreeMap::new(); for root in roots {result.extend(tree(root,cancelled)?);} Ok(result)
}
pub(crate) fn unused_name(
    path: &Path,
    directory: bool,
    reserved: &HashSet<String>,
) -> Result<PathBuf, String> {
    let leaf = path.file_name().ok_or("Path has no leaf")?;
    let (stem, extension) = if directory {
        (leaf, None)
    } else {
        (path.file_stem().unwrap_or(leaf), path.extension())
    };
    for n in 1..u32::MAX {
        let mut name = stem.to_os_string();
        name.push(format!(" ({n})"));
        if let Some(ext) = extension {
            name.push(".");
            name.push(ext);
        }
        let candidate = path.with_file_name(name);
        if !reserved.contains(&path_key(&candidate)) && matching_path(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err("No unused Keep Both name is available".into())
}

#[derive(Clone, Debug)]
pub(crate) struct PlannedFile {
    pub source: PathBuf,
    pub target: PathBuf,
    pub replace: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub files: Vec<PlannedFile>,
    pub directories: Vec<PathBuf>,
    source_directories: Vec<PathBuf>,
    new_directories: HashSet<PathBuf>,
}
impl Plan {
    pub fn target_folder(&self,source:&Path)->Option<&Path> {self.source_directories.iter().position(|path|path==source).map(|index|self.directories[index].as_path())}
    pub fn vacant_targets(&self) -> Result<bool, String> {
        for path in self
            .new_directories
            .iter()
            .chain(self.files.iter().filter(|f| !f.replace).map(|f| &f.target))
        {
            if matching_path(path)?.is_some() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// A disk-backed rollback journal, also covering reference rewrites outside the
/// merged folder. Dropping an unfinished transaction attempts rollback.
pub(crate) struct Transaction {
    directory: PathBuf,
    originals: BTreeMap<PathBuf, Option<PathBuf>>,
    created_directories: Vec<PathBuf>,
    removed_directories: Vec<PathBuf>,
    owned_files: HashSet<PathBuf>,
    changed: HashSet<PathBuf>,
    finished: bool,
}
impl Transaction {
    /// Preserve every original before deleting any member of a batch.
    pub fn delete_items(&mut self, roots: &[PathBuf], mut preserve: impl FnMut(&Path)->Result<(),String>) -> Result<(),String> {
        let mut items = BTreeMap::new();
        for root in roots { items.extend(tree(root,&||false)?); }
        for (path,info) in &items { if !info.directory { self.remember(path)?; } }
        for (path,info) in &items { if !info.directory { preserve(path)?; } }
        for (path,info) in &items { if !info.directory { self.changed.insert(path.clone()); fs::remove_file(path).map_err(|e|format!("Could not delete {}: {e}",path.display()))?; } }
        let mut dirs: Vec<_> = items.into_iter().filter(|(_,info)|info.directory).map(|(path,_)|path).collect();
        dirs.sort_by_key(|path|std::cmp::Reverse(path.components().count()));
        for path in dirs { fs::remove_dir(&path).map_err(|e|e.to_string())?; self.removed_directories.push(path); }
        Ok(())
    }
    pub fn new() -> Result<Self, String> {
        let directory = std::env::temp_dir().join(format!("baboon-merge-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        Ok(Self {
            directory,
            originals: BTreeMap::new(),
            created_directories: Vec::new(),
            removed_directories: Vec::new(),
            owned_files: HashSet::new(),
            changed: HashSet::new(),
            finished: false,
        })
    }
    pub fn remember(&mut self, path: &Path) -> Result<(), String> {
        if self.originals.contains_key(path) {
            return Ok(());
        }
        let original = if path.try_exists().map_err(|e| e.to_string())? {
            if fingerprint(path)?.readonly {
                return Err(format!("File is read-only: {}", path.display()));
            }
            let backup = self.directory.join(self.originals.len().to_string());
            copy_backup(path, &backup).map_err(|e| {
                format!(
                    "Could not prepare rollback copy for {}: {e}",
                    path.display()
                )
            })?;
            Some(backup)
        } else {
            None
        };
        self.originals.insert(path.to_owned(), original);
        Ok(())
    }
    /// Mark an external atomic reference write, after preserving its original.
    pub fn will_modify(&mut self, path: &Path) -> Result<(), String> {
        self.remember(path)?;
        self.changed.insert(path.to_owned());
        Ok(())
    }
    fn directory(&mut self, path: &Path) -> Result<(), String> {
        if path.is_dir() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            self.directory(parent)?;
        }
        fs::create_dir(path).map_err(|e| format!("Could not create {}: {e}", path.display()))?;
        self.created_directories.push(path.to_owned());
        Ok(())
    }
    pub fn apply(
        &mut self,
        plan: &Plan,
        mut preserve: impl FnMut(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        if !plan.vacant_targets()? {
            return Err("Destination changed after review; start the operation again".into());
        }
        // All recovery and rollback copies must succeed before any file changes.
        for item in &plan.files {
            self.remember(&item.target)?;
            if item.replace {
                preserve(&item.target)?;
            }
        }
        for directory in &plan.directories {
            self.directory(directory)?;
        }
        for item in &plan.files {
            if let Some(parent) = item.target.parent() {
                self.directory(parent)?;
            }
            if item.replace {
                fs::remove_file(&item.target).map_err(|e| e.to_string())?;
                self.changed.insert(item.target.clone());
            }
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&item.target)
                .map_err(|e| format!("Could not create {}: {e}", item.target.display()))?;
            self.owned_files.insert(item.target.clone());
            self.changed.insert(item.target.clone());
            let mut input = fs::File::open(&item.source).map_err(|e| e.to_string())?;
            std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
            if let Ok(modified) = input.metadata().and_then(|m| m.modified()) {
                output
                    .set_times(fs::FileTimes::new().set_modified(modified))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    pub fn remove_sources(&mut self, plan: &Plan) -> Result<(), String> {
        for item in &plan.files {
            self.remember(&item.source)?;
        }
        for item in &plan.files {
            fs::remove_file(&item.source)
                .map_err(|e| format!("Could not remove {}: {e}", item.source.display()))?;
            self.changed.insert(item.source.clone());
        }
        let mut directories = plan.source_directories.clone();
        directories.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        for directory in directories {
            if fs::read_dir(&directory)
                .map_err(|e| e.to_string())?
                .next()
                .is_none()
            {
                fs::remove_dir(&directory).map_err(|e| e.to_string())?;
                self.removed_directories.push(directory);
            }
        }
        Ok(())
    }
    pub fn rollback(&mut self, error: String) -> String {
        let mut failed = Vec::new();
        for directory in self.removed_directories.iter().rev() {
            if let Err(e) = fs::create_dir_all(directory) {
                failed.push(format!("{}: {e}", directory.display()));
            }
        }
        for (path, backup) in &self.originals {
            if !self.changed.contains(path) {
                continue;
            }
            let result = if let Some(backup) = backup {
                path.parent()
                    .map(fs::create_dir_all)
                    .transpose()
                    .map_err(|e| e.to_string())
                    .and_then(|_| copy_backup(backup, path))
            } else if self.owned_files.contains(path) && path.exists() {
                fs::remove_file(path).map_err(|e| e.to_string())
            } else {
                Ok(())
            };
            if let Err(e) = result {
                failed.push(format!("{}: {e}", path.display()));
            }
        }
        for directory in self.created_directories.iter().rev() {
            if directory.exists()
                && let Err(e) = fs::remove_dir(directory)
            {
                failed.push(format!("{}: {e}", directory.display()));
            }
        }
        self.finished = true;
        if failed.is_empty() {
            let _ = fs::remove_dir_all(&self.directory);
            format!("{error}; changes rolled back. Recovery copies were retained.")
        } else {
            format!(
                "{error}; could not restore: {}. Rollback copies retained at {}",
                failed.join("; "),
                self.directory.display()
            )
        }
    }
    pub fn commit(mut self) {
        self.finished = true;
        let _ = fs::remove_dir_all(&self.directory);
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.rollback("Folder operation interrupted".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Tree(PathBuf);
    impl Tree {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("baboon-merge-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(root.join("source")).unwrap();
            fs::create_dir_all(root.join("target")).unwrap();
            Self(root)
        }
        fn write(&self, path: &str, bytes: &[u8]) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fn scan(&self) -> Inventory {
            Inventory::scan(&self.0.join("source"), &self.0.join("target"), &|| false).unwrap()
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn failed_multi_delete_preservation_rolls_back_every_member() {
        let tree=Tree::new();tree.write("source/a",b"a");tree.write("target/b",b"b");
        let mut journal=Transaction::new().unwrap();let mut preserved=0;
        let result=journal.delete_items(&[tree.0.join("source"),tree.0.join("target")],|_|{preserved+=1;if preserved==2{Err("recovery unavailable".into())}else{Ok(())}});
        assert!(result.is_err());journal.rollback("failed".into());
        assert_eq!(fs::read(tree.0.join("source/a")).unwrap(),b"a");assert_eq!(fs::read(tree.0.join("target/b")).unwrap(),b"b");
    }

    #[test]
    fn recursive_merge_preserves_unrelated_files_and_empty_directories() {
        let tree = Tree::new();
        tree.write("source/sub/new.bin", b"new");
        tree.write("target/sub/existing.bin", b"old");
        fs::create_dir_all(tree.0.join("source/empty")).unwrap();
        let plan = tree
            .scan()
            .plan(|_| panic!("no conflicts"), &|| false)
            .unwrap();
        let mut transaction = Transaction::new().unwrap();
        transaction
            .apply(&plan, |_| panic!("nothing overwritten"))
            .unwrap();
        transaction.remove_sources(&plan).unwrap();
        transaction.commit();
        assert_eq!(fs::read(tree.0.join("target/sub/new.bin")).unwrap(), b"new");
        assert_eq!(
            fs::read(tree.0.join("target/sub/existing.bin")).unwrap(),
            b"old"
        );
        assert!(tree.0.join("target/empty").is_dir());
        assert!(!tree.0.join("source").exists());
    }
    #[test]
    fn keep_both_reserves_suffixes_and_skip_leaves_source_files() {
        let tree = Tree::new();
        for name in ["rifle.weapon", "rifle (1).weapon", "readme"] {
            tree.write(&format!("source/{name}"), b"incoming");
            tree.write(&format!("target/{name}"), b"existing");
        }
        let plan = tree
            .scan()
            .plan(
                |c| {
                    Ok(if c.incoming.file_name().unwrap() == "readme" {
                        Choice::Skip
                    } else {
                        Choice::KeepBoth
                    })
                },
                &|| false,
            )
            .unwrap();
        assert!(
            plan.files
                .iter()
                .any(|f| f.target.ends_with("rifle (2).weapon"))
        );
        let mut transaction = Transaction::new().unwrap();
        transaction
            .apply(&plan, |_| panic!("no replacements"))
            .unwrap();
        transaction.remove_sources(&plan).unwrap();
        transaction.commit();
        assert_eq!(
            fs::read(tree.0.join("target/rifle.weapon")).unwrap(),
            b"existing"
        );
        assert_eq!(fs::read(tree.0.join("source/readme")).unwrap(), b"incoming");
    }
    #[test]
    fn mismatches_cannot_replace_directories_and_keep_both_keeps_their_contents() {
        let tree = Tree::new();
        tree.write("source/item", b"file");
        tree.write("target/item/child", b"keep");
        assert!(
            tree.scan()
                .plan(
                    |c| {
                        assert!(!c.replace_allowed());
                        Ok(Choice::Replace)
                    },
                    &|| false
                )
                .is_err()
        );
        let plan = tree
            .scan()
            .plan(|_| Ok(Choice::KeepBoth), &|| false)
            .unwrap();
        assert!(plan.files[0].target.ends_with("item (1)"));
        let mut transaction = Transaction::new().unwrap();
        transaction.apply(&plan, |_| unreachable!()).unwrap();
        transaction.commit();
        assert_eq!(fs::read(tree.0.join("target/item/child")).unwrap(), b"keep");
    }
    #[test]
    fn changed_files_and_new_items_invalidate_review_and_cancel_does_not_write() {
        let tree = Tree::new();
        tree.write("source/a", b"one");
        let inventory = tree.scan();
        tree.write("source/a", b"two");
        assert!(!inventory.unchanged(&|| false).unwrap());
        let inventory = tree.scan();
        tree.write("target/added", b"new");
        assert!(!inventory.unchanged(&|| false).unwrap());
        assert!(tree.scan().plan(|_| unreachable!(), &|| true).is_err());
        assert!(!tree.0.join("target/a").exists());
    }
    #[test]
    fn failed_recovery_copy_leaves_both_trees_untouched() {
        let tree = Tree::new();
        tree.write("source/a", b"incoming");
        tree.write("target/a", b"existing");
        let original_time = fs::metadata(tree.0.join("target/a"))
            .unwrap()
            .modified()
            .unwrap();
        let plan = tree
            .scan()
            .plan(|_| Ok(Choice::Replace), &|| false)
            .unwrap();
        let mut transaction = Transaction::new().unwrap();
        assert!(
            transaction
                .apply(&plan, |_| Err("recovery disk unavailable".into()))
                .is_err()
        );
        transaction.rollback("failed".into());
        assert_eq!(
            fs::metadata(tree.0.join("target/a"))
                .unwrap()
                .modified()
                .unwrap(),
            original_time
        );
        assert_eq!(fs::read(tree.0.join("source/a")).unwrap(), b"incoming");
        assert_eq!(fs::read(tree.0.join("target/a")).unwrap(), b"existing");
    }
    #[test]
    fn rollback_restores_replacements_referrers_and_removed_empty_folders() {
        let tree = Tree::new();
        tree.write("source/a", b"incoming");
        tree.write("target/a", b"existing");
        tree.write("referrer", b"old reference");
        let original_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        fs::OpenOptions::new()
            .write(true)
            .open(tree.0.join("referrer"))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_time))
            .unwrap();
        fs::create_dir_all(tree.0.join("source/empty")).unwrap();
        let plan = tree
            .scan()
            .plan(|_| Ok(Choice::Replace), &|| false)
            .unwrap();
        let mut transaction = Transaction::new().unwrap();
        transaction.will_modify(&tree.0.join("referrer")).unwrap();
        transaction.apply(&plan, |_| Ok(())).unwrap();
        transaction.remove_sources(&plan).unwrap();
        tree.write("referrer", b"new reference");
        let result = transaction.rollback("reference rewrite failed".into());
        assert!(result.contains("changes rolled back"));
        assert_eq!(fs::read(tree.0.join("source/a")).unwrap(), b"incoming");
        assert_eq!(fs::read(tree.0.join("target/a")).unwrap(), b"existing");
        assert_eq!(fs::read(tree.0.join("referrer")).unwrap(), b"old reference");
        assert_eq!(
            fs::metadata(tree.0.join("referrer"))
                .unwrap()
                .modified()
                .unwrap(),
            original_time
        );
        assert!(tree.0.join("source/empty").is_dir());
        assert!(!tree.0.join("target/empty").exists());
    }
}
