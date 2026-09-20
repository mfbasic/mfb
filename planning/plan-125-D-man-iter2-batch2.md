# plan-125-D: man iteration 2, batch 2 — fs, strings, term, astrings, io (142 pages)

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Depends on: plan-125-C (iteration 2 is a single ordered pass; C's batch lands
before D's, and C's calibration of per-page pace and of the example ledger
format carries forward).

Landing unit: **each Phase below is independently landable and gets its own
commit.** The letter totals x-large; it is never landed as one change, and a
session that lands one phase and stops has left the tree consistent.

Iteration 2, batch 2: **text, files, and the terminal** — the packages whose
pages are read by a developer who is mid-task and landing from a search, which
is exactly the reader the page lens exists to serve.

The unit is one page; the question is *is every sentence true, and is this
page self-sufficient*. See plan-125-A §3.2.

References:

- plan-125-A §3.2/§3.3/§4.3/§5; plan-125-B's terminology table.
- `.ai/man-content.md`; plan-108-A §3 (2a) memory-vocabulary ban and its
  rewrite table — `fs` and `io` are handle packages and this ban is where
  their pages historically failed.
- Memory `string-concat-beats-list-join-in-mfb` — `s = s & ch` beats
  `List OF String` + join by ~3×; any `strings` page that advises otherwise is
  wrong in the direction a reader will believe.
- Memory `unicode-pinned-tables-vs-utf8proc` — `strings`/`astrings` Unicode
  claims (normalization, grapheme boundaries, case folding) are version-
  sensitive; probe, do not reason.
- `src/docs/spec/unicode/**` — the internals foil for `strings`/`astrings`
  (and the destination for anything cut as too internal).
- Memory `interactive-state-never-in-an-immutable-tree` and
  `.ai/canvas-threading.md` — foils for `term`, whose pages must state the
  developer-visible behavior and never the backend's threading model.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-C complete | `grep -c '^- \[ \]' planning/completed/plan-125-C-man-iter2-batch1.md` → `0` | MET 2026-09-16: `0` open; all four Commit lines filled (closed `c420fec6c`) |
| plan-125-A gate: release `mfb` at HEAD | `cargo build --release` after the last source edit | MET 2026-09-16: rebuilt after `c420fec6c` (the last source change); later commits touch only `planning/` |
| plan-125-A gate: `codex` CLI | `~/local/bin/codex --version` | MET 2026-09-16: `codex-cli 0.153.0` |
| plan-125-A gate: account can spend a request | the PONG probe | MET 2026-09-16: `PONG`; re-run before every batch |
| plan-125-A gate: no peer mid-flight in D's packages | recent commits on all refs touching `src/codegen/builtins/{io,astrings,term,strings,fs}`, then a content diff of the newest peer checkpoint against `main` on those paths | MET 2026-09-16: the only recent commits are a peer session's t3 checkpoints (whole-tree snapshots, newest `b9681abc8`); its content diff against `main` in these five packages is empty, so no prose edit is in flight |
| this branch's pages are current | content diff `HEAD...main` on the same five package paths | MET 2026-09-16: 8 lines in `astrings/gen_astrings.rs` (lowering code, no page prose) |

## 1. Goal

- **All 142 pages** through my per-page pass, one `codex exec` each, and apply.
- **Every example compiled and run** — and for `fs`, **every example rewritten
  to touch only temp paths** (plan-108-D's standing rule); a `fs` example that
  writes a real user path is a defect regardless of what it documents.
- Every parameter description and prose error claim verified.
- Every finding has a verdict; every rejection a disproving command.
- `--reconcile` exits 0 over the 142-unit list; sweeps clean for all five
  packages.
- "Belongs in spec" cuts appended to `planning/plan-125-belongs-in-spec.md` —
  `strings`/`astrings` Unicode internals are the likeliest source in the whole
  man surface.

### Non-goals (explicit constraints)

- Per plan-125-A. No cross-page reconciliation (letter G owns it).
- **No example anywhere in this letter touches a path outside a temp
  directory**, and none leaves a file behind.
- No wording churn on verified prose.

## 2. Current State

`fs` (41 pages) and `strings` (39) were reviewed by plan-108-D and -C
respectively; `term` (24) and `astrings` (15) have both had substantial churn
since (`term` 26 file-touches, `astrings` 22 —
`git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins/<pkg> | wc -l`),
and there is an **in-flight `term` change in the working tree at plan-writing**
(plan-125-A's prerequisite gate) that must land before this letter starts.

### Measured populations

| What | Count | Command |
|---|---|---|
| `fs` units | 43 | 41 pages + overview + types — `./scripts/man-census.sh --fill` |
| `strings` units | 40 | 39 + overview (no types page) |
| `term` units | 26 | 24 + overview + types |
| `astrings` units | 17 | 15 + overview + types |
| `io` units | 16 | 15 + overview (no types page) |
| **batch total** | **142** | sum |
| parameter descriptions in batch | 174 | census: fs 54, strings 64, term 30, astrings 19, io 7 |
| type descriptions in batch | 33 | census: astrings 17, term 15, fs 1 |
| `term` file-touches since plan-108 | 26 | `git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins/term \| wc -l` |
| `astrings` file-touches since plan-108 | 22 | same, `astrings` |
| `io` param descriptions | 7 for 15 pages | census PARAM-DESC `7/7` — the lowest parameter density in the batch; check that the pages that take no parameters really take none |

### Verified properties

- **`term` is mid-change at plan-writing** — VERIFIED by `git status`: 20+
  modified files under `src/codegen/builtins/term/` and
  `src/docs/spec/app/04_term-backend.md`. This letter reviews `term` *after*
  that lands, or it reviews prose that is about to move.
- **`fs` looks clean to a source grep and is not** — VERIFIED in plan-108-A:
  37 `owns` hits in `fs` are Rust module-doc lines that never render.
  Measure `fs` by rendering (`--memory-scope fs`), never by grepping the
  `.rs` files.

## 3. Design Overview

Per page: my pass → one `codex exec` → apply, at `N` concurrency with the main
thread as sole writer.

**Order:** `io` (16, smallest and the handle-vocabulary calibrator) →
`astrings` (17) → `term` (26) → `strings` (40) → `fs` (43, largest and the one
whose examples need the most care, with pace known).

**Risk concentration:**
- **`fs` examples on real paths.** The single highest-consequence defect class
  in this letter: an example a developer copy-pastes that deletes or
  overwrites something. Every `fs` example is rewritten to a temp path and run
  in a scratch project; the ledger records the temp path used.
- **Unicode claims in `strings`/`astrings`.** Memory
  `unicode-pinned-tables-vs-utf8proc` records that the pinned UCD and
  utf8proc disagree on 4,804 scalars. A normalization or grapheme claim is
  probe-verified against the binary, and where behavior is version-dependent
  the page says what *this* compiler does.
- **`term` prose vs. an in-flight backend change.** Held by the prerequisite.
- **Handle vocabulary in `fs`/`io`.** These are the packages the memory ban
  was written for. plan-108's rewrite table is applied, and the check is not
  "does it pass the grep" but "does the page still say what happens to the
  handle" — plan-108-F's recorded failure mode was deleting a true contract to
  pass a grep.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — io (16 units)

- [x] 15 function pages + overview.
- [x] Verify the open/close contract wording against plan-125-B's terminology
      table and plan-108-A's rewrite table; the page must still state what
      happens to the handle.
      Read, not grepped. `io` has no `RES` handle of its own; its lifecycle
      contract is the standard streams, and every page still states it:
      - buffering: off by default, drained on `io::flush`, before a read, when an
        append would exceed 4 KiB, and at exit (`/tmp/p125-ex/iobuf`); a failed
        drain on `setBuffered(FALSE)` is now stated as lost (bug-660);
      - standard input: the main thread reads without subscribing; a worker must
        call `thread::openStdIn` or raises `ErrInvalidContext`; at most 128
        subscribers;
      - terminal state: single-key reads restore normal input when they finish,
        and a failed restore raises `ErrInputFailed`.
      `--memory-scope io` 0 and `--scope io` 0 after the edits.
- [x] Every example compiled and run; ledger recorded.
      `STDIN_FILE=/tmp/p125-ex/io-stdin.txt ./scripts/man-run-examples.sh io --run`
      → 22/22 built and ran. Without real standard input the 10 input-reading
      examples raise `ErrEndOfFile`, a harness condition, not a page defect. The two
      rewritten `pollInput` examples were run with empty input, with data, and with
      an open pipe (`iopollex1`, `iopollex2`).

#### Phase 1 ledger — Codex iteration 2 (`planning/plan-125-findings/D-phase1/`)

Probes, all in `/tmp/p125-ex/`: `iobuf`, `ioeof`, `ioinbad`, `ioreads`, `ioreadline`,
`ioattr`, `iopipe`, `ioterm`, `iopollA`–`iopollG`, `iopollex1`, `iopollex2`,
`iobufclosed`, `iowriteclosed`, `ioflushclosed`.
Registry-data findings (a member raises an error its descriptor does not declare)
go to **bug-659** rather than prose. Scope cuts have rows 72–79 in
`planning/plan-125-belongs-in-spec.md`.

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| overview | 1 | CONFIRMED | `ioeof`: `éZtail` → `readLine` returns `tail`, the next read raises `77020003` | end-of-input sentence |
| overview | 2, 3 | CONFIRMED (scope cut) | runtime log and compiler-subscription wording | observable subscription rule (rows 72, 73) |
| overview | 4 | CONFIRMED | `stdin_broadcast.rs`: `STDIN_LOG_MAX_SUBSCRIBERS` is 128 | cap sentence |
| flush | 1 | CONFIRMED → **bug-659** | `lower_flush` raises `ErrWriteFailed`; `errors: vec![]` | — |
| flush | 2, 3 | CONFIRMED (scope cut) | write loop, `fsync`, buffer slide-back | observable failure rule (rows 77, 78) |
| flush | 4 | CONFIRMED | `app_io.rs`: in TUI mode `io::flush` drives a present | app-mode sentence |
| input | 1 | CONFIRMED → **bug-659** | `ioinbad`: `0xFF` → `77020004`; `errors: vec![]` | — |
| input | 2 | CONFIRMED | `gen_read_line_family.rs` calls `emit_console_raw_line_mode` in single-key mode | terminal-mode sentence |
| input | 3, 4 | CONFIRMED (scope cut) | broadcast log; window input pipe | rows 74, 79 |
| isBuffered | — | NO FINDINGS | `iobuf`: `FALSE`, `TRUE`, `FALSE` | — |
| isErrorTerminal | — | NO FINDINGS | `ioterm`: `FALSE` redirected | app-mode sentence aligned with its siblings |
| isInputTerminal | 1 | CONFIRMED | "no real standard streams" is inaccurate | app-mode sentence |
| isInputTerminal | 2 | CONFIRMED | `ioterm`: `FALSE` with standard input redirected | example intro "standard input is a terminal" |
| isOutputTerminal | 1 | CONFIRMED | `gen_is_terminal.rs` calls only `isatty(fd)`; `TERM` is never read | usage sentence |
| isOutputTerminal | 2 | CONFIRMED (scope cut) | "without probing a descriptor" | app-mode sentence (row 79) |
| pollInput | 1 | CONFIRMED → **bug-659** | `iopollE`: `pollInput(-1)` → `77050002`; `errors: vec![]` | — |
| pollInput | 2 | CONFIRMED | `func_poll_input.rs`: `os_poll` reloads the full timeout on each EINTR retry | timeout and signal sentences |
| pollInput | 3, 4 | CONFIRMED (example broken) | end of input counts as ready, so both examples raised `ErrEndOfFile` | rewritten; `iopollex1`/`iopollex2`: empty → `(end of input)`, data → `hello`/`h`, open pipe → nothing / `timeout` after 1.0 s |
| print | 1 | CONFIRMED | the `AttributedString` overload was missing from the intro | intro |
| print | 2 | CONFIRMED | `ioattr`: a bold `AttributedString` prints `48 69 0a` | parameter |
| print | 3, 7 | CONFIRMED | `app_io.rs`: the app-mode `fd_path` and TUI routing | console guarantee; app-mode destinations |
| print | 4 | CONFIRMED | `iopipe`: piped into `head -n 1`, producer status `141` (`SIGPIPE`) | `SIGPIPE` sentence |
| print | 5 | CONFIRMED → **bug-659** | `lower_write_family` raises `ErrWriteFailed`; `errors: vec![]` | — |
| print | 6 | CONFIRMED | `emit_append_to_buffer` keeps an exact 4096-byte fill | drain wording |
| printError | 1, 3 | CONFIRMED | `app_io.rs`: `[stderr] ` prefix; GTK appends via `g_idle_add` | app-mode sentences |
| printError | 2 | CONFIRMED | `gen_write_family.rs` keeps stderr out of the console grid; the macOS app writer routes TUI output without that exclusion | console and app TUI sentences |
| printError | 4 | CONFIRMED (scope cut) | cursor mechanics | console guarantee (row 78) |
| printError | 5 | CONFIRMED → **bug-659** | `ErrWriteFailed` undeclared | — |
| readByte | 1 | CONFIRMED → **bug-659** | `ioreads`: end of input → `77020003` | — |
| readByte | 2, 4 | CONFIRMED (scope cut) | termios flags; broadcast log | rows 75, 74 |
| readByte | 3 | CONFIRMED | `io_terminal.rs:emit_restore_stdin_terminal` has a `restore_failed` branch | restore sentence |
| readChar | 1, 2, 5 | CONFIRMED (scope cut) | lead-byte decoding; termios flags; broadcast log | rows 76, 75, 74 |
| readChar | 3 | CONFIRMED | `pollInput(0)` only reports one ready byte | `pollInput` caveat |
| readChar | 4 | CONFIRMED → **bug-659** | `ioreads`: `0xFF` and truncated `E2 82` → `77020004` | — |
| readLine | 1 | CONFIRMED | `ioreadline`: `tail\r` at end of input → `tail` (length 4), then `77020003` | trailing-CR sentence |
| readLine | 2 | CONFIRMED → **bug-659** | `ioreadline`: `0xFF` → `77020004` | — |
| readLine | 3 | CONFIRMED (scope cut) | broadcast log | row 74; termios and "leaves the terminal untouched" aligned with `readChar` and `input` |
| setBuffered | 1, 2 | CONFIRMED → **bug-660** | `iobufclosed`: stdout closed, exit `23`, nothing reported; controls `iowriteclosed` (`77020002`) and `ioflushclosed` (`77020002`) | drain-failure sentence |
| write | 1, 2 | CONFIRMED | same writer as `print` (`ioattr`) | intro sentence, parameter |
| write | 3 | CONFIRMED | `term_grid.rs`: TUI output positions text by `\n`/`\r` | TUI sentence |
| write | 4 | CONFIRMED → **bug-659** | `ErrWriteFailed` undeclared | — |
| writeError | 1–3 | CONFIRMED | as `printError` | console and app-mode sentences |
| writeError | 4 | CONFIRMED → **bug-659**; app-mode unchecked writes added to **bug-660**'s audit | `app_io.rs` fallback writes ignore the result | console guarantee |

Found while probing, not raised by a reviewer:
- **bug-658**: a recovered `pollInput(-1)` failure followed by a successful `pollInput`
  crashes the program with SIGBUS at exit (`iopollB`, `iopollE`; `iopollA`, `D`, `F`,
  `G` controls exit 0).

Acceptance: 16 units `exit 0`; `--memory-scope io` 0 unclassified; every `io`
page states its handle contract in the permitted vocabulary (read, not
grepped).

Measured at the closing commit:
- `--reconcile --letter D-phase1` → `units=16 unaccounted=0 orphans=0`;
- `--memory-scope io` 0 unclassified; `--scope io` 0;
- `io` examples 22/22 with real standard input;
- all 16 units render with none of the retired phrasings.

Bugs filed from this phase: bug-658, bug-659, bug-660. Belongs-in-spec rows: 72–79.
Commit: 6f6c05547

### Phase 2 — astrings (17 units)

- [x] 15 pages + overview + the 17-description types page.
- [x] Every byte-vs-scalar-vs-grapheme claim probe-verified; the
      `astrings`/`strings` boundary stated the same way on both sides.
      Ranges and indices are scalar throughout, and the probes hold them to it:
      `astrtie` (inclusive `[start, endIndex]`, equal-start tie-break, the two
      range errors), `astrget` (empty result, winners in insertion order),
      `astrrem` (structural match compares colour and size values), `astrbound`
      (`strings::` overloads on an `AttributedString`) and `astrmd` (flag nesting
      order, empty input). `strings::byteLen` stays the only byte measure and
      `graphemes` the only grapheme one; neither page claims otherwise.
      The boundary paragraph is now word-for-word identical on both overviews
      (rendered both, compared whitespace-normalized → identical).
- [x] Every example compiled and run; ledger recorded.
      `./scripts/man-run-examples.sh astrings --run` → 17/17 built and ran. The
      new overview example was run separately (`/tmp/p125-ex/astrov`) and its
      printed output is what the page shows.

#### Phase 2 ledger — Codex iteration 2 (`planning/plan-125-findings/D-phase2/`)

Probes, all in `/tmp/p125-ex/`: `astrbound`, `astrtie`, `astrov`, `astrget`,
`astrrem`, `astrmd`. Registry-data findings (a member raises an error its
descriptor does not declare) go to **bug-661** rather than prose; every affected
page already names both errors in its text.

The run was split by the Codex usage limit: 8 units first, then the 9 left in
`planning/plan-125-units/D-phase2-resume.txt`. Re-run rows append, so
`--reconcile` reads each unit's last row; all 17 come back `exit 0`.

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| overview | 1 | CONFIRMED | `astrbound`: `replace(s, "l", "LL")` → `**he**LLLL**o** worLLd`; `padLeft` padding unstyled | boundary paragraph, identical on both overviews: added text carries no styling |
| overview | 2 | CONFIRMED | no example rendered | example added; `astrov` printed `**hello** world`, `**hel**`, `**he**LL**o** worLd`, `hello world` |
| types | 1 | CONFIRMED | `AttrTypeNumber` registers `FontSize`, `Foreground`, `Background` | `AttrNumber` description |
| types | 2 | CONFIRMED | `astrtie`: `fontSize(-7)` → `::;-7::h` | `FontSize` description |
| addAttribute | 1 | CONFIRMED → **bug-661** | `astrtie`: `-1, 2` and `3, 2` → `77050002`; `0, 5` and empty `0, 0` → `77050001`; `errors: vec![]` | — (prose already names both errors) |
| addAttribute | 2 | CONFIRMED | `astrtie`: equal starts, size 10 then 20 → 20; 20 then 10 → 10 | tie-break sentence |
| addAttribute | 3, 4 | CONFIRMED | `__astrings_validateRange` | `start`, `endIndex` parameters |
| background | 1 | CONFIRMED | `astrtie`: `AttrNumber(n)` → `2014458966`; `toPacked(fromPacked(n.value)) = n.value` → `TRUE` | round-trip sentence |
| background | 2 | CONFIRMED | `mfb man term drawText`: `drawText(row AS Integer, column AS Integer, …)` | `drawText(row, column, value)` |
| bold | — | NO FINDINGS | — | — |
| clearAttributes | 1 | CONFIRMED → **bug-661** | derived Errors rows; prose already names both errors | — |
| font | 1 | CONFIRMED | `__astrings_font` stores `name` unchanged; reviewer probe `empty=font=` | `name` parameter |
| font | 2 | CONFIRMED | `astrtie` tie-break (shared `__astrings_isWinner`) | tie-break sentence; same wording on `background`, `fontSize` |
| fontSize | 1 | CONFIRMED | `astrtie`: `fontSize(-7)` kept | `size` parameter |
| fontSize | 2 | CONFIRMED | `astrtie`: equal starts → later wins | tie-break sentence |
| fromString | 1 | CONFIRMED | `astrtie` reuses `a` after three `addAttribute` calls and it keeps no spans | attribute sentence: the call returns a new value |
| italic | 1 | CONFIRMED | `astrtie`: empty text `0, 0` → `77050001` | shared range sentence pointing at `addAttribute`; applied to every flag constructor |
| overline | — | NO FINDINGS | — | — |
| foreground | 1 | CONFIRMED | `grep -rn AttributedString src/codegen/builtins/canvas/` → nothing; canvas `Text.text` is a `String` | canvas claim removed |
| foreground | 2 | CONFIRMED | shared `__astrings_isWinner` (`astrtie`) | tie-break sentence; `drawText(row, column, …)` and the round-trip sentence aligned with `background` |
| getAttributes | 1 | CONFIRMED | `astrget`: empty text raises `77050001` | `index` parameter |
| getAttributes | 2 | CONFIRMED | `astrget`: `empty=[]` | empty-result sentence |
| getAttributes | 3 | CONFIRMED | `astrget`: spans font/bold/font/fontSize → `flag text:Winner num:12`, the winners in insertion order | ordering sentence |
| getAttributes | 4 | CONFIRMED → **bug-661** | `errors: vec![]` with `FAIL error(77050001, …)` in the body | — |
| removeAttribute | 1 | CONFIRMED | `astrrem`: removing a different foreground keeps `num:4294901760`; the same one removes it; same for `fontSize(11)` vs `fontSize(10)` | structural-match sentence names foreground and background |
| removeAttribute | 2, 3 | CONFIRMED | `__astrings_validateRange` | `start`, `endIndex` parameters |
| underline, strike | 1 | CONFIRMED | `astrtie`: empty text `0, 0` → `77050001` | shared range sentence |
| toMarkdown | 1 | CONFIRMED | `astrmd`: `toMarkdown(fromString(""))` → `[]` | `value` parameter |
| toMarkdown | 2 | CONFIRMED (scope cut) | "canonical (enum-declaration) order" | rendered order kept, rationale dropped; `astrmd`: `**a*****b******__~~^^cd^^~~__******e*****f**` |

Measured at the closing commit:
- `--reconcile --letter D-phase2` → `units=17 unaccounted=0 orphans=0`;
- `--memory-scope astrings` 0 unclassified; `--scope astrings` 0; same for `strings`;
- `astrings` examples 17/17;
- all 17 units render with none of the retired phrasings (`higher-start-wins`,
  `x, y, value`, `canvas surface`, `enum-declaration`, `mutation members`).

Bugs filed from this phase: bug-661. Belongs-in-spec rows: none.

Acceptance: 17 units `exit 0`; sweeps clean; the `astrings`↔`strings` boundary
sentence is identical in wording on both packages' overviews.
Commit: 48f8f2c4e (8 of 17), 1f0edf0f5 (closed)

### Phase 3 — term (26 units)

- [ ] 24 pages + overview + the 15-description types page.
- [ ] No backend or threading detail on any page; anything cut goes to the
      "belongs in spec" ledger against `src/docs/spec/app/04_term-backend.md`.
- [ ] Every example compiled and run — a `term` example that leaves the
      terminal in a modified state is a defect; verify each restores it.
- [ ] Ledger recorded.

#### Phase 3 progress — paused on the Codex usage limit (boxes stay open)

14 of 26 units reviewed (`exit 0`) and applied below; the other 12 came back
`FAILED` in 3 s with the usage limit in their logs. A retry on 2026-09-20 failed the
same way: the logs name the reset as **Sep 22nd, 2026 9:27 AM**, so the remaining
units wait for that. Resume with
`./scripts/doc-review-fanout.sh --letter D-phase3 --units planning/plan-125-units/D-phase3-resume.txt --prompt planning/plan-125-prompts/man-iter2-page.txt --jobs 4 --timeout 1800`
(re-run rows append; `--reconcile` reads each unit's last row).

**Running a `term` example needs a terminal.** `scripts/man-run-examples.sh term --run`
reports 10 of 43 as failures: with stdout on a pipe `term::on` has no terminal and the
calls raise `ErrUnsupported` (`7-705-0007`, probe `/tmp/p125-ex/termsize`). That is a
harness condition, not a page defect — and the undeclared error behind it is
**bug-664**. Every example is therefore run on a real pty by
`/tmp/p125-ex/term-pty-run.py`, which opens an 80x24 pty, feeds a keystroke to
anything that waits for input, and checks the terminal was left restored:

- 43 of 43 examples: `exit=0`, no error, terminal restored — except `term::isOn#2`
  and `term::off#2`, which never enter TUI mode, so there is nothing to restore
  (both exit 0 and draw nothing, which is what those two examples are for);
- `--memory-scope term` 0 unclassified; `--scope term` 0.

Probes, all in `/tmp/p125-ex/`: `termsize`, `termov`, `termglyph`, `termattr`,
`termgeom`, `termcolor`, `termnocolor`, `termresize`, `termresize2`, plus the pty
runner `term-pty-run.py` and `verify-d3-examples.py` (which resizes the pty mid-run).

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| overview | 1 | CONFIRMED → **bug-662** | `termglyph` on a pty: `drawGlyph(0, 0, 1114112)` emits `f4 90 80 80`; `drawGlyph(2, 0, 55296)` emits `ed a0 80` | `drawGlyph` sentence: an Integer code point, values below `U+0020` skipped |
| overview | 2 | CONFIRMED | no example rendered | example added; `termov` on a pty drew the box and printed `drew on 80x24` |
| clear | 1 | CONFIRMED | `src/target/linux_gtk/app_io.rs:emit_app_term_clear` calls `g_idle_add(TERM_REDRAW_IDLE_SYMBOL)` | per-target present sentence |
| didResize | 1 | CONFIRMED | the GTK and macOS paths set the flag only when the cell extent changes | cell-dimensions wording |
| didResize | 2, 3 | CONFIRMED (examples broken) | `termresize` vs `termresize2`: with `term::sync` the resize is reported (`f9:100x30`), without it never (`resizes=[]`) | both examples rewritten as frame loops; the second reports `last seen 100x30` after the pty is resized |
| types | 1 | CONFIRMED | `term::TermSize` is registered read-only | `TermSize` description |
| types | 2, 3 | CONFIRMED | the fields are extents; `0 .. n-1` describes positions | `columns`, `rows` descriptions |
| drawHLine | — | NO FINDINGS | — | — |
| drawVLine | — | NO FINDINGS | — | — |
| drawGlyph | 1, 2 | CONFIRMED → **bug-662** | `termglyph` (above); `emit_draw_glyph` guards only `< 32` | description and `codepoint` parameter |
| drawGlyph | 3 | CONFIRMED | `emit_draw_glyph` stamps a `WIDE_TRAIL` at `column + 1` | double-width sentence (row 82) |
| drawText | 1 | CONFIRMED | the control-character branch tests the first byte `< 32` | `text` parameter |
| drawText | 2 | CONFIRMED → **bug-663** | `termattr` on a pty: bold on scalar 1 of decomposed `é` emits `e`, an attribute switch, then the mark in the next cell (`b"e\xcc\x81"` never appears); `toMarkdown` reports `e**́**` | overload paragraph states the split |
| drawBox | 1, 2 | CONFIRMED | `termgeom`: reversed corners drew; off-surface values clamp | `rowB`, `columnB` parameters |
| drawBox | 3 | CONFIRMED | `termgeom`: `drawBox(Double, 5, 5, 5, 5)` → one `╝`; the one-cell-wide box → `┐`, `│`, `┘` | degenerate-box sentence |
| fillRect | 1 | CONFIRMED | `termgeom`: `fillRect(Dark, 18, 2, 19, 4)` → 6 cells, so both corners are filled | inclusive-corners sentence |
| fillRect | 2, 3 | CONFIRMED | `termgeom`: `(-5, -5, -1, -1)` → 0 cells, `(-1, -1, 0, 0)` → 1 cell | `rowB`, `columnB` parameters |
| getForeground | 1 | CONFIRMED | `termnocolor`: reading `.alpha` without `IMPORT color` → `TYPE_UNKNOWN_VALUE` | import sentence |
| getForeground | 2 | CONFIRMED → **bug-664** | `emit_get_color` raises `ErrOutOfMemory`; `errors: vec![]` | — |
| getForeground | 3 | CONFIRMED (scope cut) | 24-bit packed storage | row 80 |
| getBackground | 1 | CONFIRMED | same as `getForeground` (`termnocolor`) | import sentence |
| getBackground | 2 | CONFIRMED | `termcolor`: `setForeground(rgba(0, 128, 255, 9))` reads back `0,128,255,255` | "red, green and blue match; alpha is always 255" |
| getBackground | 3 | CONFIRMED → **bug-664** | `errors: vec![]` with an `ErrOutOfMemory` raise site | — |
| getBackground | 4 | CONFIRMED (scope cut) | packed state | row 80 |
| getBold | — | NO FINDINGS | — | — |
| hideCursor | 1 | CONFIRMED (scope cut) | "retained surface", escape-sequence mechanics | row 81; `showCursor` aligned with it |
| hideCursor | 2, 3 | CONFIRMED | `src/target/linux_gtk/app_io.rs:emit_app_term_set_cursor` schedules a redraw itself | per-target present sentences on both cursor pages |

Bugs filed from this phase: **bug-662** (`drawGlyph` emits invalid UTF-8),
**bug-663** (`drawText` splits a grapheme when styling changes inside it),
**bug-664** (`term` members declare no errors). Belongs-in-spec rows: 80-82.

Note: the plan-125 bugs were renumbered this session — main had taken 654-657 for
unrelated bugs, so ours moved to 658-662 (`io::pollInput` crash, `io` declared
errors, `io::setBuffered` drain, `astrings` declared errors, `term::drawGlyph`).

Acceptance: 26 units `exit 0`; sweeps clean; every `term` example leaves the
terminal restored (verified by running it).
Commit: —

### Phase 4 — strings (40 units)

- [ ] 39 pages + overview.
- [ ] Every Unicode claim probe-verified; every performance hint checked
      against memory `string-concat-beats-list-join-in-mfb` before it is left
      standing.
- [ ] Every example compiled and run; ledger recorded.

Acceptance: 40 units `exit 0`; sweeps clean; no page advises an accumulation
pattern the measurement contradicts.
Commit: —

### Phase 5 — fs (43 units)

- [ ] 41 pages + overview + types page.
- [ ] **Every example rewritten to temp paths and run**; the ledger records
      the temp path per example and confirms nothing is left behind.
- [ ] Handle contract wording verified by reading, not grepping
      (`--memory-scope fs` is necessary and not sufficient).
- [ ] Ledger recorded.

Acceptance: 43 units `exit 0`; `--reconcile` exits 0 over the whole 142-unit
batch; the example ledger accounts for all 142 examples, 0 unaccounted; no
`fs` example references a path outside a temp directory
(`./scripts/man-run-examples.sh fs --run` clean, and a read-through of the 41
examples confirming it).
Commit: —

## Validation Plan

- Tests: none (man prose); pinned-text tests updated in the same commit if
  touched.
- Coverage check: `--reconcile` over the 142-unit list; example ledger
  reconciled against the census function list, 0 unaccounted.
- Runtime proof: `scripts/man-run-examples.sh <pkg> --run` for all five;
  `mfb man <pkg> --all` renders.
- Doc sync: `planning/plan-125-belongs-in-spec.md` appended (expect the
  Unicode-internals cuts here).
- Acceptance: `--fill` 100%; `--memory-scope` 0 unclassified; `--scope` 0;
  `--reconcile` 0.

## Open Decisions

- **How much Unicode does a `strings` page owe the reader?** — Recommend: the
  developer-visible rule and the one-line pointer to `mfb man unicode`; the
  table version, the pinned UCD version, and the utf8proc divergence go to
  `mfb spec unicode` via the "belongs in spec" ledger. A `strings` page that
  explains normalization forms is documenting the wrong layer.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

Two risks dominate: an `fs` example a developer runs against a real path, and
a Unicode claim in `strings`/`astrings` that is true by reasoning and false in
this binary. Both are held by *running the thing* rather than reading it, which
is the only reason iteration 2 costs what it costs.
