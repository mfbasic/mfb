# bug-93 — G1 LOW cluster: coverage anchor collisions, inline-TRAP coverage gap, stale app-mode doc

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G1). Three small,
independent LOW findings from the frontend group, batched per goal-02's
batching rule. Distinct fixes; land separately if preferred.

## 1. coverage.html anchor collisions between similarly named files

`src/coverage.rs:239-243` — `anchor()` maps every non-alphanumeric char to `-`
with no uniqueness handling, so paths differing only in punctuation
(`a/b.mfb` vs `a.b.mfb` vs `a_b.mfb` — `_` is also non-alphanumeric) share one
HTML id. The file-tree link for the second file jumps to the first file's
section, and the document carries duplicate ids.

Trigger: project with `src/a/b.mfb` and `src/a.b.mfb`; `mfb test --coverage`;
click the second file in the index → lands on the first file.

Fix: `src/doc.rs:126-145` already solves this identically-shaped problem with a
`used` set + `-2` suffixes — reuse that scheme.

## 2. Coverage instrumentation skips inline-TRAP handler bodies

`src/testing/desugar.rs:468-489` — `instrument_nested` recurses into If /
loops / Match bodies but has no arm for the `handler` block of an
`Expression::Trapped` (inline `TRAP(e) … END TRAP`), so statements inside
inline trap handlers get no `__mfb_cov_hit` slot and render Neutral
(uninstrumented) even when executed. Function-level TRAP bodies *are*
instrumented (desugar.rs:417-419), so the omission is an inconsistency, not a
policy.

Trigger: `LET x = fs::readText(p) TRAP(e)` with several handler statements →
`mfb test --coverage` shows no red/green on those lines regardless of
execution; instrumented-line totals understate.

Fix: add a Trapped arm to `instrument_nested` that instruments
`handler` like any nested block.

## 3. Stale doc comment: app mode is not macOS-only

`src/target.rs:149-152` — `NativeBackend::supports_app_mode` doc says "Only
macOS backends advertise this; the CLI rejects `-app` for any other target",
but `linux_aarch64/mod.rs:173` and `linux_x86_64/mod.rs:180` also return `true`
(GTK4 app mode), and target.rs:26-36's own `NativeBuildMode::LinuxApp` docs
describe Linux app mode as supported. Comment-only fix.
