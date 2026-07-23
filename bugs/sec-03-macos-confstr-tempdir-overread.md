# sec-03: macOS `emit_temp_directory` trusts `confstr` return as a copy count without clamping to the 4096 buffer (potential over-read)

Last updated: 2026-07-23
Effort: small (<1h)
Severity: LOW
Class: Memory-safety

Status: Open
Regression Test: (to add) a macOS runtime assertion that `fs.tempDirectory` copies
at most `TEMP_CAPACITY` bytes regardless of the `confstr` return; ideally a unit
check on the emitted sequence that a clamp branch exists.

The macOS implementation of `emit_temp_directory`
(`src/target/macos_aarch64/code.rs:599-624`) calls
`confstr(_CS_DARWIN_USER_TEMP_DIR, buf, 4096)` and then returns `confstr`'s result
minus 1 as the length the caller (`lower_fs_temp_directory_helper`) uses to size
the result String allocation and drive the byte-copy loop out of the fixed 4096
buffer. Per its contract, `confstr` returns the length **required to hold the full
string including the NUL** — which, on truncation, can be *larger* than the
supplied buffer size. Nothing here clamps the returned value back to 4096, so a
`_CS_DARWIN_USER_TEMP_DIR` longer than 4096 bytes would make the copy loop read
past the end of the buffer (and allocate/copy `confstr_ret - 1` bytes).

The single correct behavior a fix produces: the length used for the allocation and
copy is `min(confstr_ret, TEMP_CAPACITY)` (matching the Linux sibling, which
already clamps), so the copy can never exceed the buffer regardless of what
`confstr` reports.

This is **not currently reachable as an attacker-controlled overflow**:
`_CS_DARWIN_USER_TEMP_DIR` is an OS-determined per-user path (a
`/var/folders/…`-style directory), bounded well under `PATH_MAX` (~1024), and is
not influenced by app input or environment variables. It is filed as a
defense-in-depth / parity fix, not a live exploit: the guarantee "the copy is
bounded by the buffer" should hold structurally, not rest on an OS invariant about
the length of a system path.

References:

- `src/target/macos_aarch64/code.rs:599-624` (`emit_temp_directory`) — the
  unclamped `confstr` return.
- `src/target/linux_common/code.rs:803-893` (`emit_temp_directory`) — the Linux
  sibling, which DOES clamp the `TMPDIR` length loop against the passed capacity
  (`compare_registers(len, capacity); branch_ge(&fallback)`, `:859-860`). The two
  platforms diverge here; this bug closes the gap.
- `src/target/shared/code/fs/paths.rs:363-439` (`lower_fs_temp_directory_helper`)
  — the caller: allocates `TEMP_CAPACITY` (4096), calls the platform hook, then
  treats the returned register as both the String allocation size and the
  copy-loop bound (`:406-437`).
- `confstr(3)` man page — return value is the buffer size that *would be* needed,
  which may exceed `len`.
- Found during the 2026-07-23 runtime security audit (untrusted-data /
  fixed-buffer sweep); noted as a lead, verified here.

## Failing Reproduction

There is no app-level reproduction today, because `_CS_DARWIN_USER_TEMP_DIR` never
exceeds 4096 bytes on any real macOS system. The defect is structural: the emitted
sequence contains no clamp. The observable gap:

```
# macOS emit_temp_directory (code.rs:606-623), paraphrased:
#   ARG2 = 4096 (buffer size passed by the caller)
#   ARG1 = buf
#   ARG0 = _CS_DARWIN_USER_TEMP_DIR
#   confstr(ARG0, ARG1, ARG2)
#   RET  = RET - 1          <-- returned to the caller as the copy length
#                               with NO min(RET, 4096) clamp
```

Contrast: the Linux sibling caps its length loop at the passed capacity and falls
back to `/tmp` if the value does not fit (`linux_common/code.rs:859-860`), so the
copy is bounded by construction on that platform.

- Observed: the copy length is whatever `confstr` returns (minus 1), unbounded by
  the 4096 buffer in the emitted code.
- Expected: the copy length is `min(confstr_ret, 4096)` (or `confstr_ret - 1`
  clamped to `4096 - 1`), so a hypothetical over-long path truncates rather than
  over-reading.

## Root Cause

`emit_temp_directory` (macOS) forwards `confstr`'s return verbatim (after the `-1`)
to `lower_fs_temp_directory_helper`, which trusts it as an in-bounds length. On
truncation `confstr` returns a value greater than the buffer size, and no code
path reduces it. The Linux implementation avoids this because it computes the
length itself with an explicit capacity-bounded loop; the macOS path delegates the
length to `confstr` and never re-bounds it.

## Goal

- The macOS temp-directory copy length is clamped to the 4096 buffer capacity, so
  the copy cannot read past the buffer no matter what `confstr` returns.

### Non-goals (must NOT change)

- The Linux/other-platform `emit_temp_directory` behavior (already clamped).
- The `TEMP_CAPACITY` value or the caller's allocation shape in
  `lower_fs_temp_directory_helper`.
- The returned path contents for the normal (short-path) case — must stay
  byte-identical.

## Blast Radius

- `src/target/macos_aarch64/code.rs:emit_temp_directory` — fixed by this bug.
- `src/target/linux_common/code.rs:emit_temp_directory` — unaffected (already
  clamps); the reference for the correct shape.
- `src/target/shared/code/fs/paths.rs:lower_fs_temp_directory_helper` — the caller;
  a clamp could alternatively live here (bound the returned register to
  `TEMP_CAPACITY` before use), which would cover every platform hook at once — the
  more robust placement.

## Fix Design

Preferred: clamp in the shared caller `lower_fs_temp_directory_helper` immediately
after the platform hook returns — `length = min(ret, TEMP_CAPACITY)` — so any
present or future platform `emit_temp_directory` is covered by one guard, not each
platform separately. Alternatively (or additionally) clamp inside the macOS
`emit_temp_directory` right after the `-1` (`min(ret, 4096)`), mirroring the Linux
loop's `branch_ge` bound.

Rejected alternative — leaving it to the OS invariant that
`_CS_DARWIN_USER_TEMP_DIR` is short: correct today, but a fixed-buffer copy should
be bounded by the buffer, not by an external assumption; the cost is one compare.

## Phases

### Phase 1 — failing check + audit

- [ ] Add an assertion (unit on the emitted sequence, or a runtime probe with a
      stubbed `confstr` returning `> 4096`) that the copy length is clamped.
      Confirm it fails against current macOS codegen.
- [ ] Confirm no other platform hook has the same unclamped-return shape.

Acceptance: the check fails today; the audit lists each platform's clamp status.
Commit: —

### Phase 2 — the fix

- [ ] Clamp the returned length to `TEMP_CAPACITY` (preferred: in
      `lower_fs_temp_directory_helper`; or in macOS `emit_temp_directory`).

Acceptance: Phase 1 check passes; normal temp-dir resolution unchanged on both
platforms; no golden output moves.
Commit: —

### Phase 3 — validation

- [ ] Run the fs runtime suite on macOS and Linux.
- [ ] Confirm no golden/`.ncode` deltas beyond the added clamp.

Acceptance: full suite green; the only delta is the bound.
Commit: —

## Validation Plan

- Regression test(s): the clamp assertion above.
- Runtime proof: `fs.tempDirectory` still returns the correct path on macOS; a
  stubbed over-long `confstr` truncates instead of over-reading.
- Doc sync: none expected.
- Full suite: the project's fs runtime acceptance gate on both platforms.

## Summary

Low-severity, defense-in-depth parity fix: the macOS temp-directory helper trusts
`confstr`'s return as an in-bounds copy length without clamping it to the 4096
buffer, unlike the Linux sibling. Not attacker-reachable today (the path is
OS-determined and short), but a fixed-buffer copy should be bounded by the buffer.
One `min(ret, TEMP_CAPACITY)` in the shared caller closes it for every platform.
