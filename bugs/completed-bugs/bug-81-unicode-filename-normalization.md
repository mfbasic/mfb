# bug-81: unicode filename normalization (NFC/NFD) mismatch across targets

Last updated: 2026-07-10
Effort: medium (1h–2h)

Three `fs` tests probe unicode-named fixtures (`src/é日😀.txt`, `src/é日😀/`)
whose on-disk bytes were created on macOS and committed **NFD-decomposed**,
while the `.mfb` source literals are **NFC**. macOS/APFS lookup is
normalization-insensitive (the NFC probe finds the NFD-stored name), so the
tests pass there; Linux ext4 is byte-exact, so the probe misses and the tests
fail on any Linux checkout. The single correct behavior a fix produces: these
three tests pass on **both** macOS and Linux, and every MFB-*created* file is
findable by its source-literal name on both targets. The failure is
loud (test failure), not silent, but it masks a real design question about
what filename bytes MFB's fs create/write path should emit.

Recorded 2026-07-01 out of the plan-00-H x86-64 sweep triage (originally
`note-01-unicode-filename-normalization.md`; promoted to a bug 2026-07-10).

References:

- fs-spec (`mfb spec` fs section) — the cross-platform filename guarantee this
  bug touches; needs an update if the create/write path normalizes.
- `strings::normalizeNfc` unicode tables (x86 fix commit 046cdc6e) — the NFC
  machinery a fix would reuse.
- Origin: plan-00-H x86-64 backend sweep ([plan-00-H-x86-64-progress]).

## Failing Reproduction

On linux-x86_64 (or any Linux checkout), the acceptance suite runs three fs
tests against committed NFD-byte fixtures while the source literals are NFC:

```
# Linux checkout
cargo test / test-accept.sh  (the fs group)
```

- `func_fs_exists_valid`          — probes `src/é日😀.txt`
- `func_fs_fileExists_valid`      — probes `src/é日😀.txt`
- `func_fs_directoryExists_valid` — probes `src/é日😀/`

- Observed: all three fail on Linux — the NFC source literal does not
  byte-match the NFD-stored fixture name on ext4.
- Expected: all three pass on Linux and macOS.

Contrast: the same three tests pass on macOS/APFS, whose lookup is
normalization-insensitive. A file *created by an MFB program* with an NFC name
is found by that name on both targets today — the failure is confined to
fixtures created OUTSIDE the program by NFD-producing macOS tooling.

| Environment | Filesystem | Result |
| --- | --- | --- |
| macOS aarch64 | APFS (normalization-insensitive) | works ✓ |
| linux-x86_64 | ext4 (byte-exact) | fails ✗ |
| linux aarch64 | ext4 (byte-exact) | fails ✗ (would fail on any Linux) |

## Root Cause

The fixture files were committed with NFD-decomposed bytes; the `.mfb` test
source literals are NFC. The two forms are different byte strings. On ext4 a
lookup is a byte-exact `open`/`stat`, so NFC probe ≠ NFD name → ENOENT. On
APFS the FS itself folds both forms to the same file, hiding the mismatch. No
MFB runtime code is *wrong*; the bug is that the test fixtures encode a
platform-specific assumption (macOS normalization-insensitivity) that does not
hold on Linux.

## Platform facts

- macOS APFS: normalization-INSENSITIVE, normalization-PRESERVING. Both forms
  find the same file; both forms cannot coexist in one directory.
- Linux ext4: filenames are opaque bytes. NFC and NFD are different names and
  CAN coexist in the same directory. Non-UTF-8 names are legal.
- Mainstream runtimes (Python/Go/Rust/Java/Node) do NOT normalize filenames;
  they pass bytes through and document the platform difference.

## Goal

- `func_fs_exists_valid`, `func_fs_fileExists_valid`, and
  `func_fs_directoryExists_valid` pass on both macOS and Linux.
- Every file an MFB program creates with source-literal name N is findable by
  name N on both targets.

### Non-goals (must NOT change)

- **No normalization-insensitive lookup in the runtime.** On Linux a
  normalized lookup can match TWO coexisting files (NFC + NFD) with no correct
  answer for which one `fs::open` means; macOS makes that ambiguity
  unrepresentable at the FS level, but a runtime shim would inherit it. It also
  costs a `readdir` + normalized compare PER PATH COMPONENT on ENOENT (mid-path
  dirs can be NFD too) with a TOCTOU window, and fans out semantically
  (createFile when the other form exists; what bytes `listDirectory` returns;
  rename; deleteFile; invalid-UTF-8 names). Lookups must stay byte-faithful and
  O(1).
- **Do not "fix" the tests by deleting the unicode coverage** or by making them
  probe an ASCII name — the point is to exercise the real per-platform unicode
  round-trip.
- Cross-software NFD interop stays the developer's job via explicit
  `strings::normalizeNfc` / (future) `normalizeNfd` — no implicit policy is
  fully correct given both-forms-coexist on Linux.

## Blast Radius

- `func_fs_exists_valid`, `func_fs_fileExists_valid`,
  `func_fs_directoryExists_valid` — fixed by this bug (create the unicode-named
  file/dir at runtime instead of shipping NFD fixtures).
- fs create/write path (`writeText`, `createFile`, `createDir`, `rename`
  destination, temp files) — latent design question (should they NFC-normalize
  so MFB output is NFC on every platform?); in scope for the recommended
  direction below but not required to make the three tests pass.
- `listDirectory`, `deleteFile`, `open` byte-faithful lookups — unaffected;
  they must remain byte-exact per the Non-goals.

## Fix Design

Two independent pieces; the first is required, the second is the recommended
durable direction.

1. **Fix the three tests (required, quick, self-contained):** create the
   unicode-named file/dir AT RUNTIME instead of shipping NFD fixtures. Then
   they test the real per-platform round-trip guarantee (NFC literal → NFC
   bytes on disk → byte-exact match on Linux, normalization-insensitive on
   macOS) and pass on both targets regardless of decision (2).

2. **NFC-normalize on the CREATE/WRITE path (recommended durable direction):**
   `writeText`, `createFile`, `createDir`, `rename` destination, temp files, …
   emit NFC bytes on every platform, so MFB programs interoperate with
   themselves across targets (macOS preserves NFC; Linux matches byte-exact).
   Reuses the in-tree `strings::normalizeNfc` unicode tables. Needs an fs-spec
   update + both-arch validation (AArch64 goldens likely unchanged: NFC input
   passes through normalization untouched — verify byte-identity or regen).

Rejected: normalization-insensitive lookup in the runtime (see Non-goals).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Reproduce the three fs failures on a Linux checkout; confirm the ENOENT
      cause is the NFC-vs-NFD byte mismatch.
- [ ] Complete the blast-radius audit above; write each site's verdict.

Acceptance: the three tests fail on Linux for the documented reason; audit
complete.
Commit: —

### Phase 2 — the fix

- [ ] Rewrite the three tests to create the unicode-named file/dir at runtime
      (NFC literal) then probe it, removing the committed NFD fixtures.
- [ ] (Recommended) NFC-normalize the fs create/write path and update the
      fs-spec accordingly.

Acceptance: the three tests pass on macOS and Linux; byte-faithful lookups
unchanged; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any goldens the create/write-path change shifts (AArch64
      likely byte-identical — verify or regen); diff to confirm the delta is
      only the intended change.
- [ ] Run the full acceptance suite on macOS and Linux (both arches).
- [ ] Re-run the three fs tests end-to-end on every environment in the matrix.

Acceptance: full suite green on both targets; expected-output deltas are
exactly the intended change; the three tests pass everywhere they previously
failed.
Commit: —

## Validation Plan

- Regression tests: `func_fs_exists_valid`, `func_fs_fileExists_valid`,
  `func_fs_directoryExists_valid`, rewritten to create-at-runtime.
- Runtime proof: an MFB program creates `é日😀.txt` and finds it by the NFC
  literal on both macOS and Linux.
- Doc sync: fs-spec update IF the create/write path normalizes; otherwise none
  expected.
- Full suite: the project acceptance suite on macOS + linux-x86_64 +
  linux-aarch64.

## The guarantee that already holds today

A file created by an MFB program with name N is findable by name N on both
targets (NFC source literal → NFC bytes on disk → byte-exact match on Linux,
normalization-insensitive match on macOS). The failures only occur for fixture
files created OUTSIDE the program by NFD-producing tooling.

## Summary

The real risk is in decision (2): normalizing the fs create/write path touches
cross-platform filename semantics and the fs-spec, and must not slip into
normalization-insensitive *lookup* (the forbidden fix). Decision (1) — making
the three tests create their fixtures at runtime — is low-risk and makes the
suite green on both targets on its own.
