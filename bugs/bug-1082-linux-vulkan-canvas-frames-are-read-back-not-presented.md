# bug-1082: on Linux a windowed Vulkan canvas frame is read back and blitted, never presented through a swapchain

Last updated: 2026-10-12
Effort: large (3h–1d), after hardware exists
Severity: MEDIUM
Class: Other (Performance)

Status: Open — BLOCKED on hardware (no Linux box with a real Vulkan GPU) and on one design decision
Regression Test: none yet

Split out of bug-688 on 2026-10-12 (owner: bug-688 is worked on macOS and on Windows box 206;
this is its Linux remainder). On Windows a windowed Vulkan frame goes to a swapchain
(bug-688 V6). On Linux every frame, in a real window too, is rendered offscreen, copied to a
buffer, swizzled on the CPU and handed to `canvas::blitSurface`.

**The single correct behavior a fix produces:** in a real GTK window on a Linux machine with a
Vulkan GPU, a canvas frame is presented without a CPU readback (`directFrames` counts it), and
the readback path stays for headless runs, damage mode and dumps, as on Windows and Metal.

References:

- `src/codegen/runtime/canvas/vulkan_present.rs:has_present_path` — `matches!(platform.family(),
  PlatformFamily::Windows)` (`grep -n "fn has_present_path" -A2` on 2026-10-12).
- `bugs/bug-688-vulkan-canvas-lacks-bug-686-metal-parity.md`, "Linux present — BLOCKED on
  hardware", and its V6 split table: on box 206 at 1080p the readback path costs 38 ms a frame
  against about 5 ms presented.
- `.ai/remote_systems.md`, "Vulkan: one real GPU (box 206); every other driver is a CPU one".

## What blocks it

1. **Hardware.** Every reachable Linux Vulkan driver is Mesa lavapipe, where the "GPU" is the
   CPU, so neither the cost nor the gain can be measured.
2. **A design decision GTK4 forces.** GTK composites its own window, so a swapchain needs an
   X11 child window, a Wayland subsurface, or a dmabuf handed to GTK as a texture. Not decided.

## Not measured

The Linux readback cost on any real GPU. The Windows numbers above are the only evidence that
it matters.
