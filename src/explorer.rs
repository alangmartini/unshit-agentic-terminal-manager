//! Lazy filesystem listings. Only expanded directories are read, off the UI thread.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn path_for_clipboard(root: &Path, path: &Path, relative: bool) -> Result<String, String> {
    if relative {
        let path = path
            .strip_prefix(root)
            .map_err(|_| "The file is outside this workspace.".to_string())?;
        if path.as_os_str().is_empty() {
            return Ok(".".into());
        }
        Ok(path
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"))
    } else {
        std::path::absolute(path)
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|e| e.to_string())
    }
}

/// Read only the target's ancestor chain, on a worker. Map canonical paths
/// back to the workspace spelling so row ids agree with ordinary listings.
pub fn reveal_path(
    root: &Path,
    target: &Path,
) -> Result<(PathBuf, Vec<(PathBuf, Listing)>), String> {
    let canonical_root = root.canonicalize().map_err(|e| e.to_string())?;
    let canonical_target = target.canonicalize().map_err(|e| e.to_string())?;
    let relative = canonical_target
        .strip_prefix(&canonical_root)
        .map_err(|_| "The active file is outside this workspace.".to_string())?;
    let target = root.join(relative);
    let mut directory = root.to_owned();
    let mut listings = Vec::new();
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        let listing = read_directory(&directory);
        let child = directory.join(component);
        match &listing {
            Listing::Ready(entries)
                if entries.iter().any(|entry| {
                    entry.path == child && (index == components.len() - 1 || entry.directory)
                }) => {}
            Listing::Error(error) => return Err(error.clone()),
            _ => return Err("The file is no longer available in this workspace.".into()),
        }
        listings.push((directory, listing));
        directory = child;
    }
    Ok((target, listings))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub reveal_revision: u64,
    pub typeahead: String,
    pub typed_at: Option<Instant>,
}

impl Explorer {
    pub fn collapse_all(&mut self) {
        self.clear_typeahead();
        self.reveal_revision = self.reveal_revision.wrapping_add(1);
        self.expanded.clear();
        if let Some(root) = &self.root {
            self.expanded.insert(root.clone());
        }
        self.selected = self.root.clone();
    }

    pub fn set_root(&mut self, root: Option<PathBuf>) {
        if self.root != root {
            self.root = root;
            self.refresh();
        }
    }

    pub fn refresh(&mut self) {
        self.clear_typeahead();
        self.generation = self.generation.wrapping_add(1);
        self.listings.clear();
        self.expanded.clear();
        self.selected = self.root.clone();
        if let Some(root) = &self.root {
            self.expanded.insert(root.clone());
        }
    }

    pub fn clear_typeahead(&mut self) {
        self.typeahead.clear();
        self.typed_at = None;
    }

    pub fn type_to_select(&mut self, character: char, now: Instant) -> bool {
        if character.is_control() {
            return false;
        }
        let input = character.to_lowercase().collect::<String>();
        let continuing = self
            .typed_at
            .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(1));
        let cycle = continuing && self.typeahead == input;
        if !continuing || cycle {
            self.typeahead = input;
        } else {
            self.typeahead.push_str(&input);
        }
        self.typed_at = Some(now);
        let paths = self.visible_paths();
        if paths.is_empty() {
            return true;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|selected| paths.iter().position(|p| p == selected))
            .unwrap_or(0);
        let start = if continuing && !cycle {
            current
        } else {
            (current + 1) % paths.len()
        };
        for offset in 0..paths.len() {
            let path = &paths[(start + offset) % paths.len()];
            if path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .to_lowercase()
                .starts_with(&self.typeahead)
            {
                self.selected = Some(path.clone());
                break;
            }
        }
        true
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
        if generation == self.generation
            && (self.is_directory(&path)
                || self
                    .listings
                    .get(&path)
                    .is_some_and(|listing| matches!(listing.as_ref(), Listing::Loading)))
        {
            self.listings.insert(path, Arc::new(listing));
        }
    }

    /// Snapshot only visible directory listings. Arc identity lets the worker
    /// reject results superseded by a manual refresh or a newer load.
    pub fn refresh_targets(&self) -> Vec<(PathBuf, Arc<Listing>)> {
        self.listings
            .iter()
            .filter(|(path, listing)| {
                !matches!(listing.as_ref(), Listing::Loading)
                    && (Some(*path) == self.root.as_ref()
                        || (self.expanded.contains(*path)
                            && path
                                .ancestors()
                                .skip(1)
                                .take_while(|p| Some(*p) != self.root.as_deref())
                                .all(|parent| self.expanded.contains(parent))
                            && self
                                .root
                                .as_ref()
                                .is_some_and(|root| self.expanded.contains(root))))
            })
            .map(|(path, listing)| (path.clone(), listing.clone()))
            .collect()
    }

    /// Apply a directory delta without collapsing surviving folders or
    /// touching open editor buffers. Returns false for quiet/stale checks.
    pub fn update_listing(
        &mut self,
        generation: u64,
        path: PathBuf,
        previous: &Arc<Listing>,
        listing: Listing,
    ) -> bool {
        if generation != self.generation
            || !self
                .listings
                .get(&path)
                .is_some_and(|current| Arc::ptr_eq(current, previous))
            || previous.as_ref() == &listing
        {
            return false;
        }
        if let Listing::Ready(new) = &listing {
            let surviving: BTreeMap<_, _> = new
                .iter()
                .map(|entry| (&entry.path, entry.directory))
                .collect();
            // Inspect cached descendants too: an intervening read error may
            // have replaced the previous Ready listing.
            let mut removed = BTreeSet::new();
            for candidate in self
                .listings
                .keys()
                .chain(self.expanded.iter())
                .chain(self.selected.iter())
            {
                let Ok(relative) = candidate.strip_prefix(&path) else {
                    continue;
                };
                let mut components = relative.components();
                let Some(child) = components.next() else {
                    continue;
                };
                let child = path.join(child);
                let needs_directory = components.next().is_some()
                    || self.listings.contains_key(candidate)
                    || self.expanded.contains(candidate);
                if !surviving
                    .get(&child)
                    .is_some_and(|directory| !needs_directory || *directory)
                {
                    removed.insert(child);
                }
            }
            let deleted = |candidate: &Path| {
                candidate
                    .ancestors()
                    .any(|ancestor| removed.contains(ancestor))
            };
            self.expanded.retain(|p| !deleted(p));
            self.listings.retain(|p, _| !deleted(p));
            if self.selected.as_ref().is_some_and(|p| deleted(p)) {
                self.selected = Some(path.clone());
            }
        }
        self.listings.insert(path, Arc::new(listing));
        true
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
    fn copied_paths_preserve_names_and_use_portable_relative_separators() {
        let root = std::env::temp_dir().join("project");
        let file = root.join("nested folder").join("café.rs");
        assert_eq!(
            path_for_clipboard(&root, &file, true).unwrap(),
            "nested folder/café.rs"
        );
        assert_eq!(path_for_clipboard(&root, &root, true).unwrap(), ".");
        assert_eq!(
            path_for_clipboard(&root, &file, false).unwrap(),
            file.to_string_lossy()
        );
        assert!(path_for_clipboard(&root, &root.with_file_name("elsewhere"), true).is_err());
    }

    #[test]
    fn typeahead_matches_prefix_cycles_wraps_and_resets_after_pause() {
        let root = PathBuf::from("project");
        let mut explorer = Explorer::default();
        explorer.set_root(Some(root.clone()));
        explorer.accept(
            explorer.generation,
            root.clone(),
            Listing::Ready(
                ["scripts", "specs", "src", "zebra.rs"]
                    .iter()
                    .map(|name| Entry {
                        path: root.join(name),
                        name: (*name).into(),
                        directory: true,
                    })
                    .collect(),
            ),
        );
        let now = Instant::now();
        explorer.type_to_select('S', now);
        assert_eq!(explorer.selected, Some(root.join("scripts")));
        explorer.type_to_select('s', now);
        assert_eq!(explorer.selected, Some(root.join("specs")));
        explorer.type_to_select('s', now);
        assert_eq!(explorer.selected, Some(root.join("src")));
        explorer.type_to_select('r', now);
        explorer.type_to_select('c', now);
        assert_eq!(explorer.selected, Some(root.join("src")));
        explorer.type_to_select('z', now + Duration::from_secs(2));
        assert_eq!(explorer.selected, Some(root.join("zebra.rs")));
        explorer.type_to_select('s', now + Duration::from_secs(4));
        assert_eq!(
            explorer.selected,
            Some(root.join("scripts")),
            "search wraps"
        );
        explorer.type_to_select('x', now + Duration::from_secs(6));
        assert_eq!(
            explorer.selected,
            Some(root.join("scripts")),
            "no match keeps selection"
        );
        explorer.clear_typeahead();
        assert!(explorer.typeahead.is_empty());
    }

    #[test]
    fn collapse_all_keeps_root_and_cache_but_cancels_pending_reveal() {
        let root = PathBuf::from("project");
        let mut explorer = Explorer::default();
        explorer.set_root(Some(root.clone()));
        explorer.expanded.insert(root.join("deep"));
        explorer.selected = Some(root.join("deep/file"));
        let cached = Arc::new(Listing::Ready(vec![]));
        explorer.listings.insert(root.clone(), cached.clone());
        let revision = explorer.reveal_revision;
        explorer.collapse_all();
        assert_eq!(explorer.expanded, BTreeSet::from([root.clone()]));
        assert_eq!(explorer.selected, Some(root.clone()));
        assert!(Arc::ptr_eq(&cached, &explorer.listings[&root]));
        assert_ne!(revision, explorer.reveal_revision);
    }

    #[test]
    fn reveal_reads_ancestor_chain_and_rejects_outside_or_missing_files() {
        let base = std::env::temp_dir().join(format!("tm-reveal-{}", std::process::id()));
        let root = base.join("project");
        let file = root.join("nested folder").join("deep").join("example.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "example").unwrap();
        std::fs::write(base.join("outside.txt"), "outside").unwrap();
        let (selected, listings) = reveal_path(&root, &file).unwrap();
        assert_eq!(selected, file);
        assert_eq!(listings.len(), 3);
        let mut explorer = Explorer::default();
        explorer.set_root(Some(root.clone()));
        for (path, listing) in listings {
            explorer.expanded.insert(path.clone());
            explorer.accept(explorer.generation, path, listing);
        }
        assert!(explorer.visible_paths().contains(&selected));
        assert!(reveal_path(&root, &base.join("outside.txt")).is_err());
        assert!(reveal_path(&root, &root.join("missing.txt")).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn recovery_prunes_deleted_children_and_does_not_strand_pending_loads() {
        let root = PathBuf::from("project");
        let child = root.join("child");
        let mut explorer = Explorer::default();
        explorer.set_root(Some(root.clone()));
        let generation = explorer.generation;
        explorer.accept(generation, root.clone(), Listing::Error("temporary".into()));
        explorer
            .listings
            .insert(child.clone(), Arc::new(Listing::Loading));
        explorer.expanded.insert(child.clone());
        explorer.selected = Some(child.join("gone.txt"));
        explorer.accept(generation, child.clone(), Listing::Ready(vec![]));
        assert!(
            matches!(explorer.listings[&child].as_ref(), Listing::Ready(_)),
            "parent read error must not strand a pending load"
        );
        let previous = explorer.listings[&root].clone();
        assert!(explorer.update_listing(
            generation,
            root.clone(),
            &previous,
            Listing::Ready(vec![])
        ));
        assert_eq!(explorer.selected, Some(root));
        assert!(!explorer.listings.contains_key(&child));
        assert!(!explorer.expanded.contains(&child));
        assert_eq!(explorer.refresh_targets().len(), 1);
    }

    #[test]
    fn refresh_preserves_expansion_and_selection_but_prunes_deleted_subtrees() {
        let root = PathBuf::from("project");
        let folder = root.join("folder");
        let file = folder.join("file.txt");
        let listing = Listing::Ready(vec![Entry {
            path: folder.clone(),
            name: "folder".into(),
            directory: true,
        }]);
        let mut explorer = Explorer::default();
        explorer.set_root(Some(root.clone()));
        explorer.accept(explorer.generation, root.clone(), listing.clone());
        explorer.expanded.insert(folder.clone());
        explorer.accept(explorer.generation, folder.clone(), Listing::Ready(vec![]));
        explorer.selected = Some(file.clone());
        let previous = explorer.listings[&root].clone();
        assert!(!explorer.update_listing(explorer.generation, root.clone(), &previous, listing));
        assert_eq!(explorer.selected, Some(file));
        assert_eq!(explorer.refresh_targets().len(), 2);
        explorer.expanded.remove(&root);
        assert_eq!(
            explorer.refresh_targets().len(),
            1,
            "collapsed descendants are not scanned"
        );
        explorer.expanded.insert(root.clone());
        assert!(explorer.update_listing(
            explorer.generation,
            root.clone(),
            &previous,
            Listing::Ready(vec![])
        ));
        assert_eq!(explorer.selected, Some(root.clone()));
        assert!(!explorer.expanded.contains(&folder));
        assert!(!explorer.listings.contains_key(&folder));
        explorer.accept(explorer.generation, folder.clone(), Listing::Ready(vec![]));
        assert!(
            !explorer.listings.contains_key(&folder),
            "late initial loads cannot resurrect a deleted folder"
        );
        assert!(!explorer.update_listing(
            explorer.generation,
            root,
            &previous,
            Listing::Error("stale".into())
        ));
    }

    #[test]
    fn automatic_refresh_rejects_results_after_manual_refresh_or_workspace_switch() {
        let root = PathBuf::from("project");
        let mut explorer = Explorer::default();
        explorer.set_root(Some(root.clone()));
        explorer.accept(explorer.generation, root.clone(), Listing::Ready(vec![]));
        let previous = explorer.listings[&root].clone();
        let generation = explorer.generation;
        explorer.refresh();
        assert!(!explorer.update_listing(
            generation,
            root.clone(),
            &previous,
            Listing::Ready(vec![])
        ));
        explorer.set_root(Some(PathBuf::from("other")));
        assert!(!explorer.update_listing(
            generation,
            root,
            &previous,
            Listing::Error("old".into())
        ));
    }

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
