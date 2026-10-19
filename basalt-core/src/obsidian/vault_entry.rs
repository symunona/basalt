use std::{
    cmp::Reverse,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use dua_core::{Options, Order};

use super::{Error, Note, Result};

const MAX_THREADS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum VaultEntry {
    File(Note),
    Directory {
        name: String,
        path: PathBuf,
        entries: Vec<VaultEntry>,
    },
}

impl VaultEntry {
    #[allow(missing_docs)]
    pub fn name(&self) -> &str {
        match self {
            Self::Directory { name, .. } => name,
            Self::File(note) => note.name(),
        }
    }
}

impl VaultEntry {
    /// Walks `path` into a [`VaultEntry`], recursing into directories. Directories are read in
    /// parallel, so the order of entries within a directory is unspecified.
    ///
    /// `skip` is called with the path of each directory entry, relative to `path`, and whether
    /// the entry is a directory. It may be called more than once for the same entry. A skipped
    /// directory is never descended into.
    ///
    /// Symbolic links below `path` are not followed into directories: a symlinked file is
    /// included as a note, while a symlinked directory or a dangling symlink is left out. This
    /// keeps the walk inside the vault and safe from symlink loops. `path` itself may be a
    /// symlink.
    pub(crate) fn walk(
        path: &Path,
        skip: impl Fn(&Path, bool) -> bool + Send + Sync + 'static,
    ) -> Result<Self> {
        if path.is_file() {
            let note = Note::try_from((name(path, false)?, path.to_path_buf()))?;
            return Ok(VaultEntry::File(note));
        }

        let symlink_resolving_root = path.join("");
        let skip = Arc::new(skip);
        let descend = {
            let skip = Arc::clone(&skip);
            let root = symlink_resolving_root.clone();
            move |entry: &dua_core::Entry| {
                let path = entry.path();
                entry.depth == 0 || !skip(path.strip_prefix(&root).unwrap_or(&path), true)
            }
        };
        let threads = thread::available_parallelism().map_or(1, |n| n.get().min(MAX_THREADS));
        let walk = dua_core::walk(
            &symlink_resolving_root,
            threads,
            Order::Completion,
            Options::default().skip_metadata(),
            descend,
        );

        let mut children: HashMap<PathBuf, Vec<VaultEntry>> = HashMap::new();
        let mut directories = Vec::new();
        for entry in walk.flatten().filter(|entry| entry.depth > 0) {
            let entry_path = entry.path();
            let relative = entry_path
                .strip_prefix(&symlink_resolving_root)
                .unwrap_or(&entry_path);
            let is_dir = entry.file_type.is_dir();
            if skip(relative, is_dir) {
                continue;
            }

            let path = path.join(relative);
            if is_dir {
                directories.push(path);
            } else if entry.file_type.is_file() {
                let note = Note::new_unchecked(&name(&path, false)?, &path);
                push_child(&mut children, &path, VaultEntry::File(note));
            } else if let Ok(note) = Note::try_from((name(&path, false)?, path.clone())) {
                push_child(&mut children, &path, VaultEntry::File(note));
            }
        }

        directories.sort_by_key(|path| Reverse(path.components().count()));
        for directory in directories {
            let entry = VaultEntry::Directory {
                name: name(&directory, true)?,
                entries: children.remove(&directory).unwrap_or_default(),
                path: directory.clone(),
            };
            push_child(&mut children, &directory, entry);
        }

        Ok(VaultEntry::Directory {
            name: name(path, true)?,
            path: path.to_path_buf(),
            entries: children.remove(path).unwrap_or_default(),
        })
    }
}

fn name(path: &Path, is_dir: bool) -> Result<String> {
    if is_dir {
        path.file_name()
    } else {
        path.file_stem()
    }
    .map(|name| name.to_string_lossy().into_owned())
    .ok_or_else(|| Error::EmptyFileName(path.to_path_buf()))
}

fn push_child(children: &mut HashMap<PathBuf, Vec<VaultEntry>>, path: &Path, entry: VaultEntry) {
    if let Some(parent) = path.parent() {
        children
            .entry(parent.to_path_buf())
            .or_default()
            .push(entry);
    }
}

impl TryFrom<&Path> for VaultEntry {
    type Error = Error;
    fn try_from(value: &Path) -> Result<Self> {
        Self::walk(value, |_, _| false)
    }
}

#[allow(missing_docs)]
pub trait FindNote {
    #[allow(missing_docs)]
    fn find_note<'a>(&'a self, path: &Path) -> Option<&'a Note>;
}

impl FindNote for [VaultEntry] {
    fn find_note<'a>(&'a self, path: &Path) -> Option<&'a Note> {
        self.iter().find_map(|entry| entry.find_note(path))
    }
}

impl FindNote for VaultEntry {
    fn find_note<'a>(&'a self, path: &Path) -> Option<&'a Note> {
        match self {
            VaultEntry::File(note) if note.path() == path => Some(note),
            VaultEntry::Directory {
                entries,
                path: dir_path,
                ..
            } if path.starts_with(dir_path) => entries.find_note(path),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
    };
    use tempfile::tempdir;

    fn names(entries: &[VaultEntry]) -> Vec<&str> {
        let mut names: Vec<&str> = entries.iter().map(VaultEntry::name).collect();
        names.sort();
        names
    }

    #[test]
    fn directory_name_keeps_dots() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("57.02 The First Civilization");
        fs::create_dir(&path).unwrap();

        let entry = VaultEntry::try_from(path.as_path()).unwrap();

        assert_eq!(entry.name(), "57.02 The First Civilization");
    }

    #[test]
    fn note_name_drops_extension() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("57.02 Notes.md");
        fs::write(&path, "").unwrap();

        let entry = VaultEntry::try_from(path.as_path()).unwrap();

        assert_eq!(entry.name(), "57.02 Notes");
    }

    #[test]
    fn walk_does_not_descend_into_skipped_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("skipped/nested")).unwrap();
        fs::write(dir.path().join("skipped/nested/note.md"), "").unwrap();
        fs::write(dir.path().join("kept.md"), "").unwrap();

        let visited = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
        let recorded = Arc::clone(&visited);
        let skip = move |path: &Path, _| {
            recorded.lock().unwrap().push(path.to_path_buf());
            path == Path::new("skipped")
        };

        let Ok(VaultEntry::Directory { entries, .. }) = VaultEntry::walk(dir.path(), skip) else {
            panic!("expected a directory");
        };

        assert_eq!(names(&entries), ["kept"]);
        let mut visited = visited.lock().unwrap().clone();
        visited.sort();
        visited.dedup();
        assert_eq!(
            visited,
            [PathBuf::from("kept.md"), PathBuf::from("skipped")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_reads_a_symlinked_root() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("vault/folder")).unwrap();
        fs::write(dir.path().join("vault/folder/note.md"), "").unwrap();
        let link = dir.path().join("linked vault");
        symlink(dir.path().join("vault"), &link).unwrap();

        let Ok(VaultEntry::Directory { name, entries, .. }) = VaultEntry::walk(&link, |_, _| false)
        else {
            panic!("expected a directory");
        };

        assert_eq!(name, "linked vault");
        let note = link.join("folder/note.md");
        assert_eq!(
            entries.find_note(&note).map(Note::path),
            Some(note.as_path())
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_does_not_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        fs::write(dir.path().join("folder/note.md"), "").unwrap();
        // A loop back to the vault root, which would recurse forever if followed.
        symlink(dir.path(), dir.path().join("folder/loop")).unwrap();
        symlink(dir.path().join("folder"), dir.path().join("linked folder")).unwrap();
        symlink(
            dir.path().join("folder/note.md"),
            dir.path().join("linked note.md"),
        )
        .unwrap();
        symlink(
            dir.path().join("missing.md"),
            dir.path().join("dangling.md"),
        )
        .unwrap();

        let Ok(VaultEntry::Directory { entries, .. }) = VaultEntry::walk(dir.path(), |_, _| false)
        else {
            panic!("expected a directory");
        };

        assert_eq!(names(&entries), ["folder", "linked note"]);
        let folder = entries.iter().find(|entry| entry.name() == "folder");
        let Some(VaultEntry::Directory { entries, .. }) = folder else {
            panic!("expected folder to be a directory");
        };
        assert_eq!(names(entries), ["note"]);
    }
}
