# File explorer

The sidebar offers Workspaces and Explorer tabs. Ctrl+B shows or hides Explorer;
Ctrl+Shift+B toggles the sidebar. Both actions can be rebound in Settings and
Explorer is available from the command palette.

Explorer lists the active workspace folder. Click a folder to expand or collapse
it and click a file to open it in the built-in editor. Existing editor tabs are
reused, preserving unsaved buffers. Arrow keys navigate the tree; Right expands,
Left collapses or selects the parent, and Enter opens a file or toggles a folder.
Home/End select the first/last visible entry; keyboard selection scrolls into view. Escape or Tab returns keyboard input
to the active pane.

Directories load on demand on worker threads, with folders sorted before files.
Hidden and ignored entries remain accessible, including empty directories.
Directory symlinks are not traversed. Refresh clears cached listings and reloads
the root. While Explorer is visible, a background check refreshes visible folders
about once a second. Unchanged listings cause no rebuild. Additions, renames and
deletions preserve surviving expanded folders and selection; deleting a selected
entry selects its parent. Hidden panels pause checks and catch up when reopened.
Open editor buffers are never reloaded by this check. Workspace switches
clear the tree, and results from an earlier workspace or refresh are discarded.
Loading, unreadable folders, empty folders and workspaces without a folder have
explicit messages. The existing sidebar resize handle controls explorer width.

Reveal opens the active editor file in the tree: it loads and expands ancestor
folders, selects the file, and scrolls it into view. Files outside the active
workspace produce a message. Disk reads stay on a worker; switching panes or
workspaces, hiding Explorer, or moving selection cancels a pending reveal.
