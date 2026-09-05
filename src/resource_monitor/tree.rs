//! Pure process-tree attribution.
//!
//! Given the `(pid, parent pid)` table of every process on the machine and
//! a set of *roots* (session shells, the daemon, the UI), decide which root
//! each process rolls up to. The rule is **nearest known root**: a process
//! belongs to the first root met while walking up its parent chain. That
//! matters because the roots nest: ConPTY shells are children of the
//! daemon and the daemon is usually a child of the UI that started it, so
//! "everything under pid X" would file every shell under the daemon and
//! everything under the UI.
//!
//! Parent links from a process snapshot can be stale: a parent may have
//! exited and its pid been handed to an unrelated process. Callers pass a
//! creation-time lookup so an edge whose child is older than its claimed
//! parent is rejected. Unknown creation times (a process that could not be
//! opened) accept the edge, so a shell running elevated does not make its
//! whole tree vanish.

use std::collections::{HashMap, HashSet};

/// One row of the machine-wide process table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessRecord {
    pub pid: u32,
    pub parent_pid: u32,
}

/// Map every process that descends from a root to that root. Roots map to
/// themselves. Processes under no root are absent from the result.
///
/// `creation_of` returns a process's creation time in any monotone unit
/// (FILETIME 100ns ticks on Windows); `None` means unknown.
pub fn attribute(
    records: &[ProcessRecord],
    roots: &HashSet<u32>,
    creation_of: impl Fn(u32) -> Option<u64>,
) -> HashMap<u32, u32> {
    let parent_of: HashMap<u32, u32> = records.iter().map(|r| (r.pid, r.parent_pid)).collect();
    let mut result: HashMap<u32, u32> = HashMap::new();

    for record in records {
        if let Some(root) = root_for(record.pid, &parent_of, roots, &creation_of) {
            result.insert(record.pid, root);
        }
    }
    // A root that the snapshot missed (raced with its own exit) still
    // counts as itself so a caller can sample it and find out.
    for &root in roots {
        result.entry(root).or_insert(root);
    }
    result
}

fn root_for(
    pid: u32,
    parent_of: &HashMap<u32, u32>,
    roots: &HashSet<u32>,
    creation_of: &impl Fn(u32) -> Option<u64>,
) -> Option<u32> {
    let mut current = pid;
    // Bounded walk: parent tables can contain cycles (pid 0 is its own
    // parent on Windows) and a stale link could point anywhere.
    let mut seen: HashSet<u32> = HashSet::new();
    loop {
        if roots.contains(&current) {
            return Some(current);
        }
        if !seen.insert(current) {
            return None;
        }
        let parent = *parent_of.get(&current)?;
        if parent == current || parent == 0 {
            return None;
        }
        if let (Some(child_created), Some(parent_created)) =
            (creation_of(current), creation_of(parent))
        {
            if child_created < parent_created {
                // The parent pid was reused after the real parent exited.
                return None;
            }
        }
        current = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pid: u32, parent_pid: u32) -> ProcessRecord {
        ProcessRecord { pid, parent_pid }
    }

    fn roots(ids: &[u32]) -> HashSet<u32> {
        ids.iter().copied().collect()
    }

    #[test]
    fn nearest_root_wins_when_roots_nest() {
        // ui(10) -> daemon(20) -> shell(30) -> node(40) -> git(50)
        //                      -> conhost(21)
        let table = [
            rec(10, 1),
            rec(20, 10),
            rec(21, 20),
            rec(30, 20),
            rec(40, 30),
            rec(50, 40),
        ];
        let owner = attribute(&table, &roots(&[10, 20, 30]), |_| None);
        assert_eq!(owner[&10], 10);
        assert_eq!(owner[&20], 20, "daemon is its own root, not the UI's child");
        assert_eq!(owner[&21], 20, "conhost stays with the daemon");
        assert_eq!(
            owner[&30], 30,
            "the shell is its own root, not the daemon's child"
        );
        assert_eq!(owner[&40], 30);
        assert_eq!(owner[&50], 30);
        assert!(
            !owner.contains_key(&1),
            "unrelated ancestors are not attributed"
        );
    }

    #[test]
    fn processes_outside_every_tree_are_absent() {
        let table = [rec(30, 20), rec(31, 30), rec(99, 1), rec(98, 99)];
        let owner = attribute(&table, &roots(&[30]), |_| None);
        assert_eq!(owner.len(), 2);
        assert_eq!(owner[&31], 30);
    }

    #[test]
    fn a_root_missing_from_the_snapshot_still_maps_to_itself() {
        let owner = attribute(&[], &roots(&[30]), |_| None);
        assert_eq!(owner[&30], 30);
    }

    #[test]
    fn stale_parent_link_is_rejected_when_the_child_is_older() {
        // 40 claims parent 30, but 40 was created before 30: pid 30 was
        // reused by our shell after 40's real parent exited.
        let table = [rec(30, 20), rec(40, 30), rec(41, 40)];
        let created = |pid: u32| match pid {
            30 => Some(1_000),
            40 => Some(500),
            41 => Some(600),
            _ => None,
        };
        let owner = attribute(&table, &roots(&[30]), created);
        assert_eq!(
            owner.len(),
            1,
            "40 and its child 41 are not ours: {owner:?}"
        );
        assert_eq!(owner[&30], 30);
    }

    #[test]
    fn unknown_creation_times_accept_the_edge() {
        let table = [rec(30, 20), rec(40, 30)];
        let owner = attribute(&table, &roots(&[30]), |pid| (pid == 40).then_some(700));
        assert_eq!(owner[&40], 30);
    }

    #[test]
    fn cycles_and_self_parents_terminate() {
        let table = [rec(0, 0), rec(4, 0), rec(70, 71), rec(71, 70)];
        let owner = attribute(&table, &roots(&[30]), |_| None);
        assert_eq!(owner.len(), 1);
    }
}
