# bug-1059: two GPU text tests in `rt_canvas_font` fail with main's compiler on the development Mac

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Test (failing on main; cause not traced)

Status: Open
Regression Test: the two tests themselves

`cargo test --test rt_canvas_font` fails two of its 33 tests, the same two every run:
`a_transformed_text_run_reaches_the_gpu_and_matches_the_oracle` and
`text_on_the_gpu_matches_the_software_oracle`. The other 31 pass.

**Correct behavior:** both pass, as they did in the batch 20 gate.

## Failing Reproduction

Measured 2026-10-11 between 04:50 and 05:05 local time, macOS aarch64, with main's compiler
(`/tmp/main-dbg-target/release/mfb`, main at dcf790e9a) driving the test binary:

```
MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb cargo test --test rt_canvas_font
```

Six runs, six times `test result: FAILED. 31 passed; 2 failed`. The batch 21 compiler gives the
same two failures (three runs, and the batch 21 gate, `/tmp/bs-bs21-cargo-test.log`). The messages:

```
the GPU's transformed glyph run disagrees with the oracle: 1134 of 576000 pixels differ (max
channel delta 255); first at (440, 146): got [0, 0, 0, 255], want [255, 255, 255, 255]

GPU text differs from the software oracle: Mismatch { differing_pixels: 4536, max_channel_delta:
220, first: Some((112, 264, [0, 0, 0, 255], [220, 40, 160, 255])), total_pixels: 576000 }
```

In both, the GPU frame is black where the oracle has text.

## What is known

- The same test target passed in the batch 20 gate about five hours earlier, on sources that
  differ from dcf790e9a only in bug documents, one spec line and a tools oracle
  (`/tmp/bs-bs20-cargo-test.log`, every step rc=0). So the compiler did not change between the
  pass and the failure; something about the machine did.
- Not traced. Guesses, none measured: the display was asleep (`pmset -g` shows `displaysleep 10`
  and the runs were unattended at night), or another session's canvas tests were using the GPU
  at the same time (a `cargo test --test rt_canvas_pointer --test rt_canvas_hit` from another
  worktree was running when the batch 21 gate started).
- If the cause is the environment, the tests should say so (skip with a reason, or fail with a
  message that names it) rather than report a pixel mismatch.

Found during batch 21.
