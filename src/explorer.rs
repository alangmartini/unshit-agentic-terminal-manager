//! Lazy filesystem listings. Only expanded directories are read, off the UI thread.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
}

#[derive(Clone, Debug)]
pub enum Listing {
    Loading,
    Ready(Vec<Entry>),
    Error(String),
}

#[derive(Clone, Debug, Default)]
pub struct Explorer {
    pub active: bool,
    pub keyboard_focus: bool,
    pub root: Option<PathBuf>,
    pub expanded: BTreeSet<PathBuf>,
    pub listings: BTreeMap<PathBuf, Arc<Listing>>,
    pub selected: Option<PathBuf>,
    pub generation: u64,
}

impl Explorer {
    pub fn set_root(&mut self, root: Option<PathBuf>) {
        if self.root != root {
            self.root = root;
            self.refresh();
        }
    }

    pub fn refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.listings.clear();
        self.expanded.clear();
        self.selected = self.root.clone();
        if let Some(root) = &self.root {
            self.expanded.insert(root.clone());
        }
    }

    pub fn visible_paths(&self) -> Vec<PathBuf> {
        fn append(explorer: &Explorer, path: &Path, out: &mut Vec<PathBuf>) {
            out.push(path.to_owned());
            if explorer.expanded.contains(path) {
                if let Some(Listing::Ready(entries)) =
                    explorer.listings.get(path).map(AsRef::as_ref)
                {
                    for entry in entries {
                        append(explorer, &entry.path, out);
                    }
                }
            }
        }
        let mut paths = Vec::new();
        if let Some(root) = &self.root {
            append(self, root, &mut paths);
        }
        paths
    }

    pub fn is_directory(&self, path: &Path) -> bool {
        self.root.as_deref() == Some(path) || path.parent().and_then(|parent| self.listings.get(parent))
            .is_some_and(|listing| matches!(listing.as_ref(), Listing::Ready(entries) if entries.iter().any(|entry| entry.path == path && entry.directory)))
    }

    pub fn accept(&mut self, generation: u64, path: PathBuf, listing: Listing) {
        if generation == self.generation {
            self.listings.insert(path, Arc::new(listing));
        }
    }
}

pub fn read_directory(path: &Path) -> Listing {
    let read = || -> std::io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            entries.push(Entry {
                path: entry.path(),
                name: entry.file_name().to_string_lossy().into_owned(),
                // Do not traverse symlinks: a project can contain cycles.
                directory: kind.is_dir() && !kind.is_symlink(),
            });
        }
        entries.sort_by_cached_key(|e| (!e.directory, e.name.to_lowercase(), e.name.clone()));
        Ok(entries)
    };
    match read() {
        Ok(entries) => Listing::Ready(entries),
        Err(error) => Listing::Error(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_switch_and_refresh_reject_old_results() {
        let mut explorer = Explorer::default();
        explorer.set_root(Some(PathBuf::from("first")));
        let old = explorer.generation;
        explorer.set_root(Some(PathBuf::from("second")));
        explorer.accept(old, PathBuf::from("first"), Listing::Ready(vec![]));
        assert!(explorer.listings.is_empty());
        assert_eq!(explorer.expanded, BTreeSet::from([PathBuf::from("second")]));
        explorer.set_root(None);
        assert!(explorer.expanded.is_empty());
    }

    #[test]
    fn lists_folders_first_including_empty_and_hidden_entries() {
        let root = std::env::temp_dir().join(format!("tm-explorer-{}", std::process::id()));
        std::fs::create_dir_all(root.join("z-empty")).unwrap();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        std::fs::write(root.join(".hidden"), "hello").unwrap();
        let Listing::Ready(entries) = read_directory(&root) else {
            panic!("listing failed")
        };
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["z-empty", ".hidden", "a.txt"]
        );
        assert!(entries[0].directory);
        assert!(matches!(
            read_directory(&root.join("missing")),
            Listing::Error(_)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
