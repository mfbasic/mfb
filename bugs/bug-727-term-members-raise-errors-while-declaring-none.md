# bug-727: every `term` member declares no errors, yet several raise (`ErrUnsupported`, `ErrOutOfMemory`)

Last updated: 2026-09-19
Effort: small
Severity: MEDIUM
Class: Correctness (declared-error data disagrees with the lowering)

Status: Open
Regression Test: none yet — see Phase 1

All 24 `term` members are registered with `errors: vec![]`
(`grep -l 'errors: vec!\[\]' src/codegen/builtins/term/func_*.rs | wc -l` → 24), so
no `term` man page renders an Errors table. Several of them demonstrably raise:

| Member | Raises | Evidence |
|---|---|---|
| `terminalSize` (and the other calls that need a real terminal) | `ErrUnsupported` (`7-705-0007`) when standard output is not a terminal | probe `/tmp/p125-ex/termsize` run on a pipe prints `Error: 7-705-0007`; `src/codegen/term/core/term.rs:emit_unsupported` raises it |
| `getForeground`, `getBackground` | `ErrOutOfMemory` when the returned `color::Color` cannot be created | `src/codegen/term/core/term.rs:emit_get_color` → `raise_error_into(..., "ErrOutOfMemory", ...)` (`term.rs:2167`, `2274`) |
| the `term.rs:445` raise site | `ErrOutOfMemory` | confirm which member it belongs to in Phase 1 |
| **every** `term` member, in an `mfb build --app` program | `ErrWrongMode` when the program is in `app::Mode.None` or `Mode.Canvas` | `src/codegen/builtins/term/gen_shared.rs:lower_term_helper` wraps each helper in `prepend_wrong_mode_gate(…, ModeRequirement::Console)`; `canvas`, gated the same way, declares it (`func_present.rs`: `errors: vec!["ErrWrongMode", …]`). Added 2026-09-25 from the plan-125-D `term/moveTo` review |
| `terminalSize`, macOS `--app` | `ErrUnsupported` where the console raises `ErrOutOfMemory` | `src/target/macos_aarch64/app/app_io.rs:emit_app_terminal_size`: the `ARENA_ALLOC_SYMBOL` failure branch jumps to the `unsupported` label; the console `emit_terminal_size` raises `ErrOutOfMemory` there. The fix should make the two agree. Added 2026-09-25 from the plan-125-D `term/terminalSize` review |

The same programs run on a pty exit 0, so the `ErrUnsupported` path is not
theoretical — it is what every one of these calls does whenever output is
redirected, which is how they behave under `scripts/man-run-examples.sh --run`
(10 `term` examples are reported as run failures for exactly this reason).

This is the `term` instance of the class already filed for `encoding` (bug-606),
`udp` (bug-608), `math` per-overload (bug-617), `collections` (bug-641), `io`
(bug-722) and `astrings` (bug-724).

**The single correct behavior a fix produces:** every `term` member declares exactly
the errors its lowering can raise, so each page renders an Errors table. Members
that cannot raise keep `errors: vec![]`.

Found by plan-125-D Phase 3 while triaging the `getForeground` and `getBackground`
reviews.
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET size AS term::TermSize = term::terminalSize()
  term::off()
  io::print(toString(size.columns) & "x" & toString(size.rows))
END SUB
```

```
$ ./init_termsize.out | cat          # stdout is a pipe, not a terminal
Error: 7-705-0007
Operation is not supported by the implementation or platform.
$ ./target/release/mfb man term terminalSize | grep -c Errors
0
```

On a pty the same binary exits 0 and prints `80x24`.

## Root cause

Each descriptor's `errors` list is written by hand, and the shared term lowering
(`src/codegen/term/core/term.rs`) raises `ErrUnsupported` and `ErrOutOfMemory` from
helpers that were never reflected back into the per-member declarations.

## Non-goals

- Changing what any `term` member raises, or when.
- Making the console examples run without a terminal (that is a harness property;
  see the pty runner `/tmp/p125-ex/term-pty-run.py`).

## Blast-radius audit

Census all 24 members: for each, either declare the error or confirm the path cannot
be reached from that member. Include the app backends (macOS, Linux GTK, Windows),
whose surfaces raise in different places from the console one.

## Fix

Phase 1 — RED test asserting declared errors match the census. Commit:

Phase 2 — declare the errors, re-render the `term` pages; full suite and man-page
goldens. Commit:
