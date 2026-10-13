# bug-1086: the Vulkan canvas path reads a frame back without testing `vkQueueSubmit`'s or `vkQueueWaitIdle`'s result

Last updated: 2026-10-12
Effort: small
Severity: LOW
Class: Correctness

Status: Open
Regression Test: none yet

Found during `bug-staging-23` (2026-10-12) and placed in a later phase.

Metal's twin of this was fixed under bug-1024 (a command buffer that did not complete is not
read back or presented; the frame is drawn in software). Vulkan still counts such a frame as
drawn.

**The single correct behavior a fix produces:** a Vulkan frame whose submit or wait reports
an error (`VK_ERROR_DEVICE_LOST` and the like) is declined, and the caller draws it in
software, as a scene the predicate refuses.

## Evidence (by reading; no failing run exists)

- `src/codegen/runtime/canvas/vulkan.rs:emit_vulkan_draw_scene`: the calls to `vkQueueSubmit`
  and `vkQueueWaitIdle` are followed by the image copy with no compare or branch on either
  result (the `vkQueueSubmit` .. `vkQueueWaitIdle` block; read on 2026-10-12).
- `src/codegen/runtime/canvas/vulkan_present.rs` (direct present) tests `vkQueueSubmit` and
  `vkQueuePresentKHR` but not `vkQueueWaitIdle` (bs23-agent-a's reading, not re-read).

No device loss has been observed; this is the honesty gate's gap, not a measured failure.
