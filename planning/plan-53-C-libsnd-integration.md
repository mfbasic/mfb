# plan-53-C: libsnd end-to-end + close the false-green fixture

Last updated: 2026-07-17
Effort: small (<1h)
Depends on: plan-53-A (record), plan-53-B (BIND STATE)

Proves the whole feature on its motivating consumer and cleans up the false-green
plan-52-D left. `bindings/libsnd`'s `openFile`/`closeFile` — a native `sf_open`
handing back an `SNDFILE*` carrying its `SF_INFO`, and `sf_close` consuming it —
compiles as the user wrote it, and a consumer reads the `SF_INFO` fields as
`.state`.

The single outcome: **`bindings/libsnd/src/lib.mfb` compiles with `openFile` and
`closeFile` un-commented (STATE on the native funcs + `BIND STATE`), and the
previously false-green `native-resource-state-link-valid` fixture either runs
correctly or is replaced by one that does.**

References:

- `plan-53-A`, `plan-53-B` — the feature this integrates.
- `bindings/libsnd/src/lib.mfb:61-121` — `RESOURCE SoundFile`, the `LINK` block,
  `openFile` (`AS RES SoundFile STATE FileInfo` + `BIND STATE file = info`),
  `closeFile` (`RES sndfile AS SoundFile STATE FileInfo`).
- `tests/syntax/resources/native-resource-state-link-valid` — the **false-green**
  fixture plan-52-D added: it compiles (a wrapper `EXPORT FUNC` setting
  `.state.frames = 1024` on a native resource) but would corrupt at runtime, because
  a native resource had no STATE slot. Committed in `1bf52515`. Must be fixed.
- `planning/old-plans/plan-52-D-stateful-returns.md` Phase 3 — the checkbox
  "Confirm bindings/libsnd's openFile *wrapper shape* compiles" that this supersedes.

## 1. Goal

- `bindings/libsnd` builds to a `.mfp` with `openFile`/`closeFile` un-commented.
- A cross-package consumer binds `RES snd AS SoundFile STATE FileInfo =
  libsnd::openFile(path)` and reads `snd.state.samplerate` etc. — the values
  `sf_open` wrote — with no corruption.
- `native-resource-state-link-valid` is corrected: either promoted to an
  rt-behavior fixture that runs and reads real state, or its wrapper reworked to the
  BIND STATE shape. The tree no longer contains a fixture that compiles-but-corrupts.
- The `.mfp` carries `SoundFile STATE FileInfo` in `openFile`'s exported signature
  (kind-11, plan-52-D) and a consumer recovers it.

### Non-goals (explicit constraints)

- **Running against the real libsndfile.** `libsnd`'s vendored library may not be
  present in CI; the *compile* + `.mfp` round-trip is the acceptance here, plus a
  runtime read against an exercisable stand-in native library (`libsqlite3`, as the
  other native-resource fixtures use) proving the STATE marshalling end-to-end. If
  libsndfile is available, run it; do not gate CI on it.
- **New feature work** — A and B own the feature. This is integration + cleanup.

## 2. Current State

- libsnd's `openFile`/`closeFile` are commented out (the user un-commented them,
  which is what surfaced the gap). With them un-commented, the build fails:
  `MFB_PARSE_UNEXPECTED_TOKEN` on `AS RES SoundFile STATE FileInfo` and `BIND
  requires a direction: BIND IN` on `BIND STATE` (verified 2026-07-17).
- `native-resource-state-link-valid` (committed) is a package whose wrapper
  `openTagged(...) AS RES SfFile STATE FileInfo` does `f.state.frames = 1024` on a
  native resource. It builds; it was never run. Demonstrated to corrupt: the
  equivalent shape with a `String` STATE field + a real native call fails
  `7-701-0001 Allocation failed`.

## 3. Design Overview

Mechanical once A and B land:

1. **libsnd**: with A (record) and B (`BIND STATE`) in place, the un-commented
   `openFile`/`closeFile` parse and codegen. Build the package; confirm the `.mfp`.
2. **Consumer fixture**: a new `tests/rt-behavior/resources/` fixture (or reuse a
   sqlite3-backed stand-in) that opens a stateful native resource, reads `.state`,
   and closes — reading real marshalled values.
3. **Fix the false green**: convert `native-resource-state-link-valid` to the BIND
   STATE shape and give it a `.run` golden (so it is actually executed), OR delete
   it in favor of the new rt-behavior fixture. Either way, no compile-only "valid"
   fixture for a shape that corrupts at runtime remains.

## Compatibility / Format Impact

- libsnd's `.mfp` gains `openFile`/`closeFile` exports with a kind-11 STATE
  signature. No format change beyond plan-52-D's kind 11.

## Phases

### Phase 1 — libsnd compiles + fix the false green

- [ ] Build `bindings/libsnd` with `openFile`/`closeFile` un-commented; confirm the
      `.mfp` and that `pkg info` shows `openFile … AS RES SoundFile STATE FileInfo`.
- [ ] Replace/repair `tests/syntax/resources/native-resource-state-link-valid`: move
      to `tests/rt-behavior/resources/` as a runnable fixture using the BIND STATE
      shape against an exercisable native library, with a `.run` golden asserting
      real state values; or delete it if the new consumer fixture covers it.
- [ ] Cross-package consumer fixture: bind `RES … STATE …` from an imported native
      producer and read `.state`.

Acceptance: libsnd builds; `pkg info` shows the stateful export; the consumer
fixture reads the native-populated state at runtime (real values, no corruption);
no compile-only fixture for a corrupting shape remains.
Commit: —

### Phase 2 — validation + spec + archive

- [ ] `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`
      all green.
- [ ] `./mfb spec language native-libraries` + §15.5 document native stateful
      resources end-to-end (declaration → BIND STATE → consumer read).
- [ ] Update `planning/res.md` §3.4 (constructor-attached STATE is now expressible)
      and the plan-52 memory note. Archive plan-53-A/B/C to `planning/old-plans/`.

Acceptance: full suite green; specs current; plans archived.
Commit: —

## Validation Plan

- Tests: the cross-package consumer (runtime read of real state); the repaired
  fixture; libsnd's build.
- Runtime proof: **the whole point.** A consumer prints the `SF_INFO`/stand-in
  values the native call wrote, distinguishable from defaults. `samplerate=<real>`,
  not `0`.
- Doc sync: `./mfb spec language native-libraries`, `resource-management` §15.5,
  `res.md`, memory.
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **Keep or delete `native-resource-state-link-valid`.** Recommend **convert to
  runnable** (BIND STATE + `.run` golden) rather than delete — it documents the
  native-LINK-resource-with-STATE shape, which is worth a named fixture; it just
  must actually run.

## Summary

Pure integration and cleanup: A and B make the feature; this proves it on libsnd
(the reason the feature exists) and removes the plan-52-D fixture that compiled a
corrupting shape and called it valid. The lasting lesson, folded into the plan-52
memory: a resource fixture with no `.run` golden proves only that it builds — which
for a runtime-corrupting shape is worse than no fixture.
