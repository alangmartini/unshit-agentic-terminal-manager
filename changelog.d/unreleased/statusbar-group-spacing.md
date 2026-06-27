### Fixed

- The bottom status bar no longer renders the unreadable token `k/sutf-8`. The left and right status groups were laid flush against each other (`.statusbar` is `justify-content: flex-start; gap: 0`), so the left group's last item (`↓ 0.0 k/s`) collided with the right group's first (`utf-8`). A flex spacer (`.sb-spacer`, matching the settings status bar) is now inserted between the two groups, pushing the right group to the far edge as intended.
