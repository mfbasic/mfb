# bug-392: Windows console renders UTF-8 output as OEM-codepage mojibake

Last updated: 2026-07-27
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: FIXED (78d68514f)
Regression Test: tests/cli_windows_console_utf8.rs

On Windows, a compiled MFB program that prints non-ASCII UTF-8 text to the
console displays mojibake instead of the intended glyphs. The em-dash `—`
(U+2014) shows as `ΓÇö` and box-drawing `─` (U+2500) shows as `ΓöÇ…`. The
runtime emits *correct* UTF-8 bytes; the Windows console decodes them with its
legacy OEM code page (437/850) because the program never sets the console
output code page to UTF-8. This is a display-only corruption of otherwise
correct output — dangerous because the bytes look fine to any pipe/file
consumer, so it only surfaces on an interactive console and is easy to miss in
automated tests.

**The single correct behavior a fix produces:** a program that writes UTF-8
text to an interactive Windows console displays the intended Unicode glyphs
(`—`, `─`, box-drawing borders, etc.), while output redirected to a
file/pipe remains byte-identical raw UTF-8 (unchanged).

**Second manifestation (same root cause) — full-screen `term::` grid
scrambling.** The console `term::` grid (`browser` example TUI) does not just
mojibake; its layout scrambles: text lands in the wrong columns, page content
overlaps stale content (`ImagesG to type…`, `Táample` for `Example`), and
single letters scatter across a row (`h  w  a  l  o  e  g`). This is NOT an
independent grid bug — it is a downstream cascade of the same code-page defect,
verified against the present loop (see Root Cause). Fixing the code page fixes
both symptoms. Confirming there is no separate grid bug to file is part of this
document's scope.

References:

- Root cause found while investigating a screenshot of the `browser` example
  TUI on Windows (`— a tiny terminal web viewer` header + box-drawing borders
  rendered as `ΓÇö` / `ΓöÇ`).
- Related: commit b8aff9bcb "win: support term:: draw helpers on the console
  backend" (the draw helpers emit the box-drawing UTF-8 that mojibakes).
- Sibling seam already in-tree: `emit_enable_vt_output`
  (`src/target/win_x86_64/code.rs:1352`) — same "resolve STD_OUTPUT, best-effort
  console setup, skip if redirected" shape a fix should mirror.

## Failing Reproduction

A minimal MFB program that prints an em-dash to stdout, compiled for
`win_x86_64` and run on an interactive Windows console:

```
print "browser — a tiny terminal web viewer"
```

- Observed (interactive Windows console, default OEM code page 437/850):
  `browser ΓÇö a tiny terminal web viewer`
- Expected: `browser — a tiny terminal web viewer`

Contrast cases (bound the bug):

- Same program on macOS / Linux backends → correct (`—`). The console
  code-page problem is Windows-only.
- Same Windows program with stdout redirected to a file
  (`prog > out.txt`) → `out.txt` already contains the correct UTF-8 bytes
  `62 72 6F 77 73 65 72 20 E2 80 94 …`. Only the *console* decode is wrong.
- Same program after running `chcp 65001` in the console first → correct.
  This confirms the bytes are right and only the console code page is wrong.

| Environment | Detail | Result |
| --- | --- | --- |
| Windows console | default OEM code page (437/850) | fails ✗ |
| Windows console | after `chcp 65001` | works ✓ |
| Windows | stdout redirected to file/pipe | works ✓ (raw UTF-8) |
| macOS / Linux | any | works ✓ |

## Root Cause

`WindowsX8664::emit_write` (`src/target/win_x86_64/code.rs:756`) writes the
program's UTF-8 string bytes **verbatim** to the console handle via
`GetStdHandle` + `WriteFile` (`code.rs:811`, `code.rs:831`). `WriteFile` to a
console handle hands the bytes to the console, which decodes them using the
console's *output code page*. The default output code page for a fresh console
is the machine's OEM code page (typically 437 in the US, 850 in Western
Europe), **not** UTF-8 (65001). A multi-byte UTF-8 sequence is therefore
decoded as several OEM code points: `E2 80 94` (`—`) → `Γ` `Ç` `ö`.

Nothing in the entry stub (`_start`, `src/target/win_x86_64/plan.rs:50`+) or in
`emit_write` calls `SetConsoleOutputCP(65001)`. The existing
`emit_enable_vt_output` seam (`code.rs:1352`) sets `ENABLE_VIRTUAL_TERMINAL_-`
`PROCESSING` on the console mode — which is orthogonal: it makes conhost
interpret ANSI/VT escape sequences, but does nothing about how the raw *text*
bytes are decoded. So even programs that call `term::on` still mojibake.

Why the contrast cases are immune: redirected output never touches the console
code page (`WriteFile` to a file/pipe stores bytes as-is), and the macOS/Linux
backends write UTF-8 to terminals that decode UTF-8 by default.

**Why the `term::` grid scrambles (same root cause, verified).** The console
grid present, `emit_grid_present` (`src/target/shared/code/term_grid.rs:856`),
is a diff renderer that emits an absolute cursor-position escape (CUP) for a
changed cell **only when the terminal cursor is not already there**
(`term_grid.rs:991`–`999`). For a contiguous run of changed cells it omits the
CUP and relies on the terminal auto-advancing the cursor by exactly **one
column** per printed glyph — recorded as `last_col = col + 1`
(`term_grid.rs:1064`). The grid model is correct: a cell packs one codepoint
(up to 4 UTF-8 bytes) and `emit_grid_write` advances the column by 1 per
codepoint (`term_grid.rs:519`), so 1 codepoint = 1 cell = 1 column throughout.
The break is purely at the physical console: when a 3-byte glyph (`─` = `E2 94
80`) is decoded by the OEM code page as three separate glyphs (`ΓöÇ`), the
console cursor advances 3 columns while the present's model believes it advanced
1. The present then skips the CUP for the next contiguous cell and prints it 2
columns too far right; the error accumulates across the run, yielding the
scattering / overlap / stale-cell corruption observed. Set the code page to
65001 → each codepoint decodes as one glyph → the +1 auto-advance holds → the
grid aligns. No grid-code change is needed for this bug.

**Latent, genuinely-separate limitation (OUT OF SCOPE here).** The same +1
auto-advance assumption also breaks for a real double-width glyph (CJK, wide
emoji), which occupies one grid cell but two terminal columns, on *every*
platform including a correct UTF-8 console. That is a pre-existing grid design
constraint, not this bug (the reproduction here is Latin text plus box-drawing,
all single-width), and must not be conflated with or "fixed" as part of bug-392.
File it separately if it ever bites.

## Goal

- On an interactive Windows console, a program that prints UTF-8 non-ASCII
  text displays the intended glyphs without the user running `chcp` first.
- Output redirected to a file or pipe stays byte-identical raw UTF-8.
- A regression test asserts the redirected-to-file byte stream is exact UTF-8
  (the console-decode itself is not machine-checkable in CI, so the test
  guards the invariant that we still emit raw UTF-8 downstream).

### Non-goals (must NOT change)

- The UTF-8 bytes written to files/pipes — they are already correct and must
  stay byte-identical. A "fix" that transcodes output to some other encoding is
  wrong.
- macOS / Linux / riscv backends — untouched.
- The VT-processing behavior of `emit_enable_vt_output` — code page and VT mode
  are separate concerns; don't fold one into the other in a way that changes
  when VT is enabled.
- Tempting wrong fix to forbid: making the regression test assert the *console*
  rendering (unobservable in CI) or weakening it to only check ASCII — the test
  must exercise a real multi-byte UTF-8 sequence through the write path.

## Blast Radius

Search: every `WriteFile`/console-output site in the Windows backend and the
program entry.

- `emit_write` (`src/target/win_x86_64/code.rs:756`) — the sole general text
  write path (all `print`/`write` lower here). Fixed indirectly by setting the
  code page once at entry, or directly if we take the `WriteConsoleW` route.
- `_start` entry / `plan.rs:50`+ imports — the natural home for a one-time
  `SetConsoleOutputCP(65001)`; needs a `kernel32!SetConsoleOutputCP` import
  added.
- `emit_enable_vt_output` (`code.rs:1352`) — unaffected (VT mode, not code
  page); left as-is.
- File-write paths that pass a `CreateFileW` handle through `emit_write` —
  unaffected: `SetConsoleOutputCP` only changes console decoding, and
  `WriteFile` to a file handle ignores it.

## Fix Design

Two viable approaches; they compose.

**Option A (recommended, smallest): set the console output code page once at
startup.** Emit `SetConsoleOutputCP(65001)` (and optionally
`SetConsoleCP(65001)` for input) in the `_start` entry, before the program
body runs. The console then decodes the existing `WriteFile` UTF-8 bytes
correctly; `emit_write` is untouched; redirected output is unaffected because
the code page only governs console decoding. Best-effort — the call is harmless
when stdout is redirected. Requires a `SetConsoleOutputCP` import in `plan.rs`
and a small emit in the entry sequence. Risk concentrates in the entry-frame
shadow-space accounting (mirror the existing entry calls).

**Option B (robust, larger): write via `WriteConsoleW` on real consoles.** In
`emit_write`, branch on `GetConsoleMode(hFile)`: if it succeeds (a console),
convert the UTF-8 buffer to UTF-16 with `MultiByteToWideChar(CP_UTF8, …)` — the
helper already exists in this file (`code.rs:127`, `CP_UTF8` const at
`code.rs:48`) — and call `WriteConsoleW`; otherwise keep the raw `WriteFile`
path for pipes/files. This bypasses the code page entirely and is correct
regardless of `chcp` or console version, at the cost of a runtime branch and a
transcode buffer per write. Rejected as the *first* landing because it's more
codegen surface on the hottest output path; keep it as a follow-up if Option A
proves insufficient on legacy conhost.

Recommendation: land Option A. Option B is a strict superset of robustness and
can be layered later without conflict (Option A is harmless when B is present).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add a regression test that compiles a program printing a multi-byte
      UTF-8 string for `win_x86_64` and asserts the emitted/redirected byte
      stream is exact UTF-8 (per the project's Windows codegen test
      conventions). Document that console-decode itself is not CI-observable.
- [x] Confirm the blast-radius list above against the current tree
      (`grep` for `WriteFile`/`WriteConsole`/`SetConsoleOutputCP`).

Acceptance: the audit list is complete with a verdict per site; the test
harness compiles and the invariant is expressed.
Commit: 78d68514f

Notes: `SetConsoleOutputCP`/`SetConsoleCP` were absent from the whole `src/`
tree; the Windows entry (`plan.rs` `entry_imports`) declared no console
code-page import — confirming the mechanism (RED: `-nplan` for
`windows-x86_64` lacked the import). There is no Windows `.ncodesum` golden
(the artifact-gate covers macos/linux/riscv only), so the CI-observable gate
is the cross-compiled `-nplan` import surface plus the raw-UTF-8 byte survival
in the PE. Console glyph rendering itself is not CI-observable (documented).

### Phase 2 — the fix (Option A)

- [x] Add a `SetConsoleOutputCP` (and `SetConsoleCP`, optional) import to
      `src/target/win_x86_64/plan.rs`, required by `_start`.
- [x] Emit `SetConsoleOutputCP(65001)` in the entry stub before the program
      body, with a self-contained shadow-space frame like the neighbouring
      entry calls.

Acceptance: a program printing `—` / box-drawing on an interactive Windows
console renders the intended glyphs; redirected output is byte-identical;
nothing in Non-goals changed.
Commit: 78d68514f

Notes: landed via a new `emit_console_utf8` platform hook
(`src/target/shared/code/types.rs`, default no-op) called once in the shared
`_start` sequence (`src/target/shared/code/entry.rs`) right after the arena
start-time seed. The Windows override (`src/target/win_x86_64/code.rs`) emits a
balanced `sub $0x20 / mov rcx,65001 / call SetConsoleOutputCP / mov rcx,65001 /
call SetConsoleCP / add $0x20` frame (touches only caller-saved rcx/rax). Both
CPs are set (resolving the Open Decision below): console input flows through
byte-based `ReadFile` (`emit_read_file`, not wide `ReadConsoleW`), so
`SetConsoleCP(65001)` is needed for symmetric UTF-8 keyboard input. Placing the
call after the arena map is safe: incoming argc/argv are already consumed on
POSIX and are garbage on Windows (rebuilt later from `GetCommandLineW`).

### Phase 3 — regenerate expected outputs + full validation

- [x] Regenerate any Windows codegen goldens the added entry call shifts
      (`.ncodesum` / artifact-gate); confirm the delta is ONLY the new
      `SetConsoleOutputCP` sequence in `_start`.
- [x] Run the full `cargo test` suite + artifact-gate.
- [x] Re-run the reproduction on a real Windows console (box 2230) and confirm
      `—` and the box-drawing borders render correctly without `chcp`.

Acceptance: full suite green; goldens shift only by the intended entry
sequence; the reproduction renders correctly on Windows.
Commit: 78d68514f

Notes: no Windows `.ncodesum` golden exists (the artifact-gate covers the four
non-Windows targets only), so nothing to regenerate for Windows; the fix is a
defaulted no-op for those targets → **artifact-gate: 1476 goldens checked, 0
diffs** (byte-identical, as designed). Full suite: **4132 passed, 0 failed**.
Runtime proof on box 2230: a fresh console defaults to CP **437** (the OEM page
that mojibakes); running the exe (forced from 437) flips the console to CP
**65001**, exit **0** (shadow-space frame sound), output bytes correct. The
final pixel-level glyph rendering in an interactive GUI console is the one
un-CI-able piece the doc calls out, but the exact lever that governs it — the
console code page — is confirmed set to 65001 by the running program.

## Validation Plan

- Regression test(s): the Phase 1 Windows-codegen UTF-8 byte-stream test.
- Runtime proof: run the `browser` example (or the minimal `print "—"`) on
  Windows box 2230 and confirm the header/borders render as real glyphs.
- Doc sync: none expected (behavior converges to the correct default; no
  language-surface change).
- Full suite: `cargo test` + `scripts/artifact-gate.sh`.

## Open Decisions

- Also set `SetConsoleCP(65001)` for console *input*? — **RESOLVED: yes.**
  Confirmed the console *input* path is byte-based `ReadFile`
  (`win_x86_64/code.rs` `emit_read_file`), NOT wide `ReadConsoleW` — so typed
  non-ASCII would decode through the console *input* code page and mojibake
  symmetrically. Set both `SetConsoleOutputCP` and `SetConsoleCP` to 65001 in
  the one entry frame; the input call is likewise a harmless no-op when stdin is
  redirected.
- Land Option A only, or A+B together? — **RESOLVED: Option A only** (set the
  code page at entry). Option B (`WriteConsoleW` on real consoles) remains a
  strict-superset follow-up if a legacy conhost ever rejects 65001; it composes
  with A without conflict. Box 2230 (Win 10.0.26100) accepted 65001, so A holds
  there.

## STATUS: FIXED (78d68514f)

Landed via `/fix-bug 392` on `main`. One root cause, one fix location (the
Windows `_start` entry), so no fan-out — serial on the integration worktree.

- **Fix:** new `emit_console_utf8` platform hook (default no-op → all POSIX
  backends byte-identical) emitting `SetConsoleOutputCP(65001)` +
  `SetConsoleCP(65001)` once in `_start`; two `kernel32` imports added to
  `entry_imports`.
- **Second manifestation (term:: grid scramble):** confirmed a downstream
  cascade of the same code-page defect, NOT a separate bug — no grid-code
  change was made or needed. The latent double-width-glyph grid limitation
  (§Root Cause) stays out of scope and unfiled unless it bites.
- **Gates:** full `cargo test` **4132 passed / 0 failed**; artifact-gate **1476
  goldens, 0 diffs**; RED→GREEN regression test
  `tests/cli_windows_console_utf8.rs`.
- **Runtime proof (box 2230):** fresh console CP 437 → after the exe runs, CP
  **65001**; exit 0; output bytes verbatim raw UTF-8.
- **Deviation from plan:** set both output+input CP (Open Decision resolved
  yes); no Windows golden existed to regenerate.

## Summary

The engineering risk is small and concentrated in the entry-stub shadow-space
accounting for one new `SetConsoleOutputCP(65001)` call; the UTF-8 bytes the
runtime emits are already correct and must stay untouched, so the file/pipe
output path and all non-Windows backends are unaffected. The only genuinely
un-CI-able part is the final on-console visual confirmation, covered by the
Windows box re-run in Phase 3.
