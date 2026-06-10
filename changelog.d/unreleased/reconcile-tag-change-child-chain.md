### Fixed

- Reconciler: when a matched child's tag changed, the replacement node was built but the child chain was stitched with the old (deallocated) NodeId, silently truncating the sibling chain. This blanked the entire content column after closing settings (Esc or repeated `Ctrl+,`) whenever an element inside the settings page had been clicked, and rendered keybind rows/filter input blank on the restyled Keybinds page. `reconcile_inner` now returns the live NodeId and `reconcile_children` stitches that id; covered by keyed + unkeyed tag-change regression tests in `unshit-core`.
