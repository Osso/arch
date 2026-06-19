//! Filesystem snapshot + diff for capturing what an opaque installer (a
//! `deploy.sh`) writes. We walk a set of install-root trees in parallel,
//! recording per-file identity, then diff a before/after pair to recover the
//! set of files the installer created or modified. This is how a project with
//! a `deploy.sh` but no PKGBUILD still produces a tracked package: the diff is
//! the file list.
//!
//! Benchmarked on this machine: ~1.1M files across the default install roots
//! snapshot in well under a second; a whole-disk walk (7.5M files) in ~1.8s.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use jwalk::WalkDir;

/// Per-file identity. A change in any field between two snapshots means the
/// installer touched the file. `ino` catches replace-via-rename (new inode at
/// the same path); `mtime_ns`/`size` catch in-place rewrites.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileMeta {
    pub mtime_ns: i64,
    pub size: u64,
    pub ino: u64,
}

impl FileMeta {
    fn from_metadata(md: &std::fs::Metadata) -> Self {
        Self {
            mtime_ns: md.mtime() * 1_000_000_000 + md.mtime_nsec(),
            size: md.size(),
            ino: md.ino(),
        }
    }
}

/// A point-in-time inventory of regular files and symlinks under some roots.
pub struct Snapshot {
    files: HashMap<PathBuf, FileMeta>,
}

impl Snapshot {
    /// Walk `roots` in parallel, recording every regular file and symlink
    /// (directories are structural, not tracked). `excludes` prunes any path
    /// under a listed prefix (build/cache dirs that must never be tracked).
    pub fn capture(roots: &[PathBuf], excludes: &[PathBuf]) -> Self {
        let mut files = HashMap::new();
        for root in roots {
            if !root.exists() {
                continue;
            }
            collect_root(root, excludes, &mut files);
        }
        Self { files }
    }

    /// Paths that are new or changed relative to `before` — i.e. exactly what
    /// the installer wrote. Sorted for deterministic packaging output.
    pub fn added_or_changed(&self, before: &Snapshot) -> Vec<PathBuf> {
        let mut changed: Vec<PathBuf> = self
            .files
            .iter()
            .filter(|(path, meta)| before.files.get(*path).is_none_or(|old| old != *meta))
            .map(|(path, _)| path.clone())
            .collect();
        changed.sort();
        changed
    }
}

fn collect_root(root: &Path, excludes: &[PathBuf], files: &mut HashMap<PathBuf, FileMeta>) {
    for entry in WalkDir::new(root).skip_hidden(false).follow_links(false) {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if is_excluded(&path, excludes) {
            continue;
        }
        // lstat (not stat): track symlinks as themselves, never follow them.
        let Ok(md) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if md.is_dir() {
            continue;
        }
        files.insert(path, FileMeta::from_metadata(&md));
    }
}

fn is_excluded(path: &Path, excludes: &[PathBuf]) -> bool {
    excludes.iter().any(|prefix| path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn diff_reports_created_and_modified_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let a = root.join("a.txt");
        let b = root.join("sub/b.txt");
        fs::write(&a, "one").unwrap();

        let before = Snapshot::capture(&[root.clone()], &[]);

        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(&b, "new file").unwrap(); // created
        fs::write(&a, "one-changed-longer").unwrap(); // modified (size+mtime)

        let after = Snapshot::capture(&[root.clone()], &[]);
        let changed = after.added_or_changed(&before);

        assert!(changed.contains(&a), "modified file should be in diff");
        assert!(changed.contains(&b), "created file should be in diff");
    }

    #[test]
    fn excludes_prune_subtrees() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let cache = root.join("cache");
        fs::create_dir_all(&cache).unwrap();

        let before = Snapshot::capture(&[root.clone()], &[cache.clone()]);
        fs::write(cache.join("junk.tmp"), "noise").unwrap();
        let after = Snapshot::capture(&[root.clone()], &[cache.clone()]);

        assert!(
            after.added_or_changed(&before).is_empty(),
            "writes under an excluded prefix must not appear in the diff"
        );
    }
}
