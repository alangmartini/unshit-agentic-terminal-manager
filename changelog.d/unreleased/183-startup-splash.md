### Added

- The window now appears and responds while the GPU is still starting, instead
  of waiting for it. GPU adapter and device creation costs over a second on a
  cold start and none of it can be skipped, but everything needed to know *what*
  to draw -- the stylesheet, the fonts, the element tree, the layout -- is ready
  long before that. The window is painted from the real, already-laid-out tree
  using the platform's own 2D drawing, then swapped for the GPU surface once it
  is ready.

  The placeholder is the app's own geometry in the app's own colors, not a
  generic spinner: every element that has a background contributes a rectangle,
  clipped and composited exactly as the layout says. It has no gradients,
  rounded corners, shadows or terminal cells, so it reads as the application
  mid-assembly rather than as a finished frame -- which is the honest thing for
  it to look like.
