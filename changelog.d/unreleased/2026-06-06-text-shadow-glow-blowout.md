# Fix: text-shadow glow blew out to a bright smear on Windows

## Fixed

- The `text-shadow` glow rendered as a blown-out bright smear (the active
  workspace name and the prompt) on the Windows subpixel text path. The
  subpixel text shader output color premultiplied by coverage but **not** by the
  source alpha, while its pipeline blends premultiplied (`src = One`). That is a
  no-op for opaque text (the only case before `text-shadow`), but the glow's
  stacked low-alpha copies accumulated rgb far faster than alpha and clipped to a
  bright halo. The shader now folds `color.a` into the premultiplied output, so
  the glow composites correctly (its intensity tracks the shadow's alpha) and any
  translucent text on the subpixel path is correctly composited rather than
  over-bright. Opaque text is byte-identical. Guarded by a render test asserting
  the glow's intensity scales with the shadow alpha (the bug decoupled them).
