### Changed

- The software/CPU-renderer fallback now renders **box-shadows** (outer and inset), restoring panel depth so the non-GPU path looks much closer to the GPU-accelerated one. The lite quad shader (`quad_software.wgsl`) was expanded with the full shader's shadow math — outer-spread expansion in the vertex stage, the tanh-Gaussian outer/inset shadow passes, and shadow compositing behind the rect — while staying within software adapters' 60-component varying budget (it now uses ~36 of 60; gradients and `mask-image` remain omitted). The GPU path and its full shader are unchanged.
