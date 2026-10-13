# A persisted Vulkan pipeline cache shared by every MFBASIC program

Status: an idea, not a plan. Recorded 2026-10-12 from bug-996 (owner: write the idea down).

## The problem it answers

On Intel's Vulkan driver (box 206, UHD 630) `vkCreateGraphicsPipelines` for the canvas
runtime's one pipeline takes about 37 s the first time an executable NAME runs, and about
70 ms the second time (`bugs/repro/bug-996/coldstart.mfb`, measured 2026-10-12). The driver
keeps its own compiled-shader cache per application, so the same SPIR-V under a new name is
cold again: every newly built program, and every user's first launch of any MFBASIC app,
pays the compile.

bug-996 takes the compile off the first frame's path (frames draw in software until the
pipeline exists). That hides the wait; it does not shorten it. The GPU still does not draw
for those 37 s.

## The idea

The runtime keeps its own `VkPipelineCache` and persists it, in one place per user that
EVERY MFBASIC program reads and writes:

- at pipeline creation, load a blob from an MFBASIC-wide per-user cache directory (not
  `os::appCachePath`, which is per application and would be cold for every new program),
  check its header (`vendorID`, `deviceID`, `pipelineCacheUUID`) and hand it to
  `vkCreatePipelineCache`; a missing, corrupt or foreign blob is ignored, never an error;
- pass that cache to `vkCreateGraphicsPipelines` (today the argument is 0,
  `src/codegen/runtime/canvas/vulkan.rs:emit_vulkan_pipeline`);
- after a successful build, write `vkGetPipelineCacheData` back with an atomic rename.

The compile is then paid once per driver version and shader version, not once per
executable.

## What is not known

- **Whether Intel's driver honours an application cache blob across executables at all.**
  This decides the whole idea, and it is one throwaway probe on box 206: program A saves the
  blob, program B (a new name) loads it, time B's pipeline creation. Not run.
- Where the directory is on each platform, and who may delete it.
- Two programs starting together: the read is safe, the write needs the atomic rename and a
  rule for which blob wins.
- The blob's key when the shader changes (the SPIR-V's hash in the file name, or trust the
  driver's own validation).
- Whether lavapipe and other drivers gain or lose anything.
- Metal has the same shape (the pipeline is compiled from MSL at first use) and is not
  measured slow; `MTLBinaryArchive` would be its counterpart if it ever is.

## Related

- `bugs/completed/bug-996-vulkan-first-frame-waits-25s-for-a-cold-uber-shader-pipeline-compile.md`
  (its Fix Design piece 2 is this idea; piece 3, a smaller shader, is a separate question).
- `mfb spec app canvas`, "Which renderer draws is chosen once, at the first present".
