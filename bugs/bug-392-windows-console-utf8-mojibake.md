# bug-392: Windows console renders UTF-8 output as OEM-codepage mojibake

Last updated: 2026-07-27
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: <tests/ filename — added in Phase 1>

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

- [ ] Add a regression test that compiles a program printing a multi-byte
      UTF-8 string for `win_x86_64` and asserts the emitted/redirected byte
      stream is exact UTF-8 (per the project's Windows codegen test
      conventions). Document that console-decode itself is not CI-observable.
- [ ] Confirm the blast-radius list above against the current tree
      (`grep` for `WriteFile`/`WriteConsole`/`SetConsoleOutputCP`).

Acceptance: the audit list is complete with a verdict per site; the test
harness compiles and the invariant is expressed.
Commit: —

### Phase 2 — the fix (Option A)

- [ ] Add a `SetConsoleOutputCP` (and `SetConsoleCP`, optional) import to
      `src/target/win_x86_64/plan.rs`, required by `_start`.
- [ ] Emit `SetConsoleOutputCP(65001)` in the entry stub before the program
      body, with a self-contained shadow-space frame like the neighbouring
      entry calls.

Acceptance: a program printing `—` / box-drawing on an interactive Windows
console renders the intended glyphs; redirected output is byte-identical;
nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any Windows codegen goldens the added entry call shifts
      (`.ncodesum` / artifact-gate); confirm the delta is ONLY the new
      `SetConsoleOutputCP` sequence in `_start`.
- [ ] Run the full `cargo test` suite + artifact-gate.
- [ ] Re-run the reproduction on a real Windows console (box 2230) and confirm
      `—` and the box-drawing borders render correctly without `chcp`.

Acceptance: full suite green; goldens shift only by the intended entry
sequence; the reproduction renders correctly on Windows.
Commit: —

## Validation Plan

- Regression test(s): the Phase 1 Windows-codegen UTF-8 byte-stream test.
- Runtime proof: run the `browser` example (or the minimal `print "—"`) on
  Windows box 2230 and confirm the header/borders render as real glyphs.
- Doc sync: none expected (behavior converges to the correct default; no
  language-surface change).
- Full suite: `cargo test` + `scripts/artifact-gate.sh`.

## Open Decisions

- Also set `SetConsoleCP(65001)` for console *input*? — recommended yes for
  symmetry (UTF-8 keyboard input), but input already flows through the
  wide/`ReadConsole` path in places; confirm before adding. (§Fix Design)
- Land Option A only, or A+B together? — recommended A now, B as a tracked
  follow-up. (§Fix Design)

## Summary

The engineering risk is small and concentrated in the entry-stub shadow-space
accounting for one new `SetConsoleOutputCP(65001)` call; the UTF-8 bytes the
runtime emits are already correct and must stay untouched, so the file/pipe
output path and all non-Windows backends are unaffected. The only genuinely
un-CI-able part is the final on-console visual confirmation, covered by the
Windows box re-run in Phase 3.
