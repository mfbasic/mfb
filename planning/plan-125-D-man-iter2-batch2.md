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

**Reviewer gates — replace plan-125-A's `codex` CLI and Codex PONG rows (owner,
2026-09-25):** the Claude Code CLI (`claude --version`), and the account can spend a
request: `cd /tmp && printf 'Reply with exactly: PONG\n' | claude -p --model
claude-fable-5-1 --no-session-persistence` → `PONG`, re-run before every batch. Run it
**from `/tmp`**: inside `.claude/worktrees/P-*` the follow-plan Stop hook keeps the
session from ending and the probe never returns.


| Must be true | Command | Status |
|---|---|---|
| plan-125-C complete | `grep -c '^- \[ \]' planning/completed/plan-125-C-man-iter2-batch1.md` → `0` | MET 2026-09-16: `0` open; all four Commit lines filled (closed `c420fec6c`) |
| plan-125-A gate: release `mfb` at HEAD | `cargo build --release` after the last source edit | MET 2026-10-26: rebuilt after the fast-forward to `d58b5397a` (`target/release/mfb` `Oct 26 06:43`) |
| ~~plan-125-A gate: `codex` CLI~~ (superseded 2026-09-25) | `~/local/bin/codex --version` | MET 2026-09-16: `codex-cli 0.153.0` — Phases 1, 2 and 14 of Phase 3's 26 units ran on it |
| reviewer gate: Claude Code CLI | `claude --version` | MET 2026-10-26: `2.1.310 (Claude Code)` |
| reviewer gate: account can spend a request | the Fable PONG probe above, from `/tmp` | MET 2026-10-26: `PONG`, exit 0 (before the `D-phase4-current` batch); re-run before every batch |
| plan-125-A gate: no peer mid-flight in D's packages | recent commits on all refs touching `src/codegen/builtins/{io,astrings,term,strings,fs}`, then a content diff of the newest peer checkpoint against `main` on those paths | MET 2026-09-16: the only recent commits are a peer session's t3 checkpoints (whole-tree snapshots, newest `b9681abc8`); its content diff against `main` in these five packages is empty, so no prose edit is in flight. Re-checked 2026-10-26: `git status --porcelain` on these paths is empty in every live worktree; the only unmerged commits touching them are t3 checkpoint snapshots, and the newest (`d1c6d393c`, 2026-10-17) is OLDER content than main (diffing main against it removes main's `strings::lineBreaks`), so it is not an edit in flight |
| this branch's pages are current | content diff `HEAD...main` on the same five package paths | MET 2026-10-26: `git diff HEAD...main -- src/codegen/builtins/{io,astrings,term,strings,fs} \| wc -l` → `0` after the fast-forward |

## 1. Goal

- **All 144 pages** through my per-page pass, one Fable 5.1 review each, and apply.
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
| `strings` units | 42 | 41 + overview (no types page) — corrected 2026-09-25 from 40: `./scripts/man-census.sh --fill strings` → `PAGES 41` |
| `term` units | 26 | 24 + overview + types |
| `astrings` units | 17 | 15 + overview + types |
| `io` units | 16 | 15 + overview (no types page) |
| **batch total** | **144** | sum (was 142 before the `strings` correction) |
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

Per page: my pass → one Fable 5.1 review → apply, at `N` concurrency with the main
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
        drain on `setBuffered(FALSE)` is now stated as lost (bug-723);
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
go to **bug-722** rather than prose. Scope cuts have rows 72–79 in
`planning/plan-125-belongs-in-spec.md`.

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| overview | 1 | CONFIRMED | `ioeof`: `éZtail` → `readLine` returns `tail`, the next read raises `77020003` | end-of-input sentence |
| overview | 2, 3 | CONFIRMED (scope cut) | runtime log and compiler-subscription wording | observable subscription rule (rows 72, 73) |
| overview | 4 | CONFIRMED | `stdin_broadcast.rs`: `STDIN_LOG_MAX_SUBSCRIBERS` is 128 | cap sentence |
| flush | 1 | CONFIRMED → **bug-722** | `lower_flush` raises `ErrWriteFailed`; `errors: vec![]` | — |
| flush | 2, 3 | CONFIRMED (scope cut) | write loop, `fsync`, buffer slide-back | observable failure rule (rows 77, 78) |
| flush | 4 | CONFIRMED | `app_io.rs`: in TUI mode `io::flush` drives a present | app-mode sentence |
| input | 1 | CONFIRMED → **bug-722** | `ioinbad`: `0xFF` → `77020004`; `errors: vec![]` | — |
| input | 2 | CONFIRMED | `gen_read_line_family.rs` calls `emit_console_raw_line_mode` in single-key mode | terminal-mode sentence |
| input | 3, 4 | CONFIRMED (scope cut) | broadcast log; window input pipe | rows 74, 79 |
| isBuffered | — | NO FINDINGS | `iobuf`: `FALSE`, `TRUE`, `FALSE` | — |
| isErrorTerminal | — | NO FINDINGS | `ioterm`: `FALSE` redirected | app-mode sentence aligned with its siblings |
| isInputTerminal | 1 | CONFIRMED | "no real standard streams" is inaccurate | app-mode sentence |
| isInputTerminal | 2 | CONFIRMED | `ioterm`: `FALSE` with standard input redirected | example intro "standard input is a terminal" |
| isOutputTerminal | 1 | CONFIRMED | `gen_is_terminal.rs` calls only `isatty(fd)`; `TERM` is never read | usage sentence |
| isOutputTerminal | 2 | CONFIRMED (scope cut) | "without probing a descriptor" | app-mode sentence (row 79) |
| pollInput | 1 | CONFIRMED → **bug-722** | `iopollE`: `pollInput(-1)` → `77050002`; `errors: vec![]` | — |
| pollInput | 2 | CONFIRMED | `func_poll_input.rs`: `os_poll` reloads the full timeout on each EINTR retry | timeout and signal sentences |
| pollInput | 3, 4 | CONFIRMED (example broken) | end of input counts as ready, so both examples raised `ErrEndOfFile` | rewritten; `iopollex1`/`iopollex2`: empty → `(end of input)`, data → `hello`/`h`, open pipe → nothing / `timeout` after 1.0 s |
| print | 1 | CONFIRMED | the `AttributedString` overload was missing from the intro | intro |
| print | 2 | CONFIRMED | `ioattr`: a bold `AttributedString` prints `48 69 0a` | parameter |
| print | 3, 7 | CONFIRMED | `app_io.rs`: the app-mode `fd_path` and TUI routing | console guarantee; app-mode destinations |
| print | 4 | CONFIRMED | `iopipe`: piped into `head -n 1`, producer status `141` (`SIGPIPE`) | `SIGPIPE` sentence |
| print | 5 | CONFIRMED → **bug-722** | `lower_write_family` raises `ErrWriteFailed`; `errors: vec![]` | — |
| print | 6 | CONFIRMED | `emit_append_to_buffer` keeps an exact 4096-byte fill | drain wording |
| printError | 1, 3 | CONFIRMED | `app_io.rs`: `[stderr] ` prefix; GTK appends via `g_idle_add` | app-mode sentences |
| printError | 2 | CONFIRMED | `gen_write_family.rs` keeps stderr out of the console grid; the macOS app writer routes TUI output without that exclusion | console and app TUI sentences |
| printError | 4 | CONFIRMED (scope cut) | cursor mechanics | console guarantee (row 78) |
| printError | 5 | CONFIRMED → **bug-722** | `ErrWriteFailed` undeclared | — |
| readByte | 1 | CONFIRMED → **bug-722** | `ioreads`: end of input → `77020003` | — |
| readByte | 2, 4 | CONFIRMED (scope cut) | termios flags; broadcast log | rows 75, 74 |
| readByte | 3 | CONFIRMED | `io_terminal.rs:emit_restore_stdin_terminal` has a `restore_failed` branch | restore sentence |
| readChar | 1, 2, 5 | CONFIRMED (scope cut) | lead-byte decoding; termios flags; broadcast log | rows 76, 75, 74 |
| readChar | 3 | CONFIRMED | `pollInput(0)` only reports one ready byte | `pollInput` caveat |
| readChar | 4 | CONFIRMED → **bug-722** | `ioreads`: `0xFF` and truncated `E2 82` → `77020004` | — |
| readLine | 1 | CONFIRMED | `ioreadline`: `tail\r` at end of input → `tail` (length 4), then `77020003` | trailing-CR sentence |
| readLine | 2 | CONFIRMED → **bug-722** | `ioreadline`: `0xFF` → `77020004` | — |
| readLine | 3 | CONFIRMED (scope cut) | broadcast log | row 74; termios and "leaves the terminal untouched" aligned with `readChar` and `input` |
| setBuffered | 1, 2 | CONFIRMED → **bug-723** | `iobufclosed`: stdout closed, exit `23`, nothing reported; controls `iowriteclosed` (`77020002`) and `ioflushclosed` (`77020002`) | drain-failure sentence |
| write | 1, 2 | CONFIRMED | same writer as `print` (`ioattr`) | intro sentence, parameter |
| write | 3 | CONFIRMED | `term_grid.rs`: TUI output positions text by `\n`/`\r` | TUI sentence |
| write | 4 | CONFIRMED → **bug-722** | `ErrWriteFailed` undeclared | — |
| writeError | 1–3 | CONFIRMED | as `printError` | console and app-mode sentences |
| writeError | 4 | CONFIRMED → **bug-722**; app-mode unchecked writes added to **bug-723**'s audit | `app_io.rs` fallback writes ignore the result | console guarantee |

Found while probing, not raised by a reviewer:
- **bug-742**: a recovered `pollInput(-1)` failure followed by a successful `pollInput`
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

Bugs filed from this phase: bug-742, bug-722, bug-723. Belongs-in-spec rows: 72–79.
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
descriptor does not declare) go to **bug-724** rather than prose; every affected
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
| addAttribute | 1 | CONFIRMED → **bug-724** | `astrtie`: `-1, 2` and `3, 2` → `77050002`; `0, 5` and empty `0, 0` → `77050001`; `errors: vec![]` | — (prose already names both errors) |
| addAttribute | 2 | CONFIRMED | `astrtie`: equal starts, size 10 then 20 → 20; 20 then 10 → 10 | tie-break sentence |
| addAttribute | 3, 4 | CONFIRMED | `__astrings_validateRange` | `start`, `endIndex` parameters |
| background | 1 | CONFIRMED | `astrtie`: `AttrNumber(n)` → `2014458966`; `toPacked(fromPacked(n.value)) = n.value` → `TRUE` | round-trip sentence |
| background | 2 | CONFIRMED | `mfb man term drawText`: `drawText(row AS Integer, column AS Integer, …)` | `drawText(row, column, value)` |
| bold | — | NO FINDINGS | — | — |
| clearAttributes | 1 | CONFIRMED → **bug-724** | derived Errors rows; prose already names both errors | — |
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
| getAttributes | 4 | CONFIRMED → **bug-724** | `errors: vec![]` with `FAIL error(77050001, …)` in the body | — |
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

Bugs filed from this phase: bug-724. Belongs-in-spec rows: none.

Acceptance: 17 units `exit 0`; sweeps clean; the `astrings`↔`strings` boundary
sentence is identical in wording on both packages' overviews.
Commit: 48f8f2c4e (8 of 17), 1f0edf0f5 (closed)

### Phase 3 — term (26 units)

- [x] 24 pages + overview + the 15-description types page.
      `--reconcile --letter D-phase3 --units planning/plan-125-units/D-phase3.txt`
      → `units=26 unaccounted=0 orphans=0` (14 Codex rows, 12 Fable rows).
- [x] No backend or threading detail on any page; anything cut goes to the
      "belongs in spec" ledger against `src/docs/spec/app/04_term-backend.md`.
      Cuts: rows 80-82 (Codex) and 83-84 (Fable: the active-flag sentence, the
      four setters' current-attribute-state sentence). `--scope term` →
      `internals-vocabulary hits: 0`; `--memory-scope term` → `unclassified … 0`.
- [x] Every example compiled and run — a `term` example that leaves the
      terminal in a modified state is a defect; verify each restores it.
      `/tmp/p125-tools/build-examples.py term` → 44 built (43 function-page
      examples + the overview's); `/tmp/p125-tools/pty-run-all.sh` → 44 `exit=0`,
      43 `entered=True restored=True`, and `off#2` `entered=False` (it never enters
      TUI mode — that is the example). `isOn#1` never calls `term::off` and is
      restored by the runtime at exit, as the rewritten `off` page now says.
- [x] Ledger recorded (below; the Fable rows are marked "(Fable)").

#### Phase 3 progress — the Codex half (first 14 units)

14 of 26 units reviewed (`exit 0`) and applied below; the other 12 came back
`FAILED` in 3 s with the usage limit in their logs. A retry on 2026-09-20 failed the
same way: the logs name the reset as **Sep 22nd, 2026 9:27 AM**, so the remaining
units wait for that. Resume with
`./scripts/doc-review-fanout.sh --letter D-phase3 --units planning/plan-125-units/D-phase3-resume.txt --prompt planning/plan-125-prompts/man-iter2-page.txt --jobs 4 --timeout 1800`
(re-run rows append; `--reconcile` reads each unit's last row).

**Resume on Fable 5.1 (owner, 2026-09-25).** The 12 remaining units run on the Fable
reviewer with the same command — the harness now calls `claude -p --model
claude-fable-5-1` — so Phase 3's ledger mixes two reviewers: the 14 rows above are
Codex (`OpenAI Codex v…` banners in `D-phase3/manifest.tsv`), the resumed ones Fable
(`Claude Code v…/claude-fable-5-1`). Record which reviewer produced each row.

**Running a `term` example needs a terminal.** `scripts/man-run-examples.sh term --run`
reports 10 of 43 as failures: with stdout on a pipe `term::on` has no terminal and the
calls raise `ErrUnsupported` (`7-705-0007`, probe `/tmp/p125-ex/termsize`). That is a
harness condition, not a page defect — and the undeclared error behind it is
**bug-727**. Every example is therefore run on a real pty by
`/tmp/p125-ex/term-pty-run.py`, which opens an 80x24 pty, feeds a keystroke to
anything that waits for input, and checks the terminal was left restored. (The
macOS `/tmp` sweep deleted that runner and the probes before the resume; it was
rebuilt as `/tmp/p125-tools/term-pty-run.py`, and the closing example run above
uses it. The measurements below are the 2026-09-19 Codex-half run.)

- 43 of 43 examples: `exit=0`, no error, terminal restored — except `term::isOn#2`
  and `term::off#2`, which never enter TUI mode, so there is nothing to restore
  (both exit 0 and draw nothing, which is what those two examples are for);
- `--memory-scope term` 0 unclassified; `--scope term` 0.

Probes, all in `/tmp/p125-ex/`: `termsize`, `termov`, `termglyph`, `termattr`,
`termgeom`, `termcolor`, `termnocolor`, `termresize`, `termresize2`, plus the pty
runner `term-pty-run.py` and `verify-d3-examples.py` (which resizes the pty mid-run).

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| overview | 1 | CONFIRMED → **bug-725** | `termglyph` on a pty: `drawGlyph(0, 0, 1114112)` emits `f4 90 80 80`; `drawGlyph(2, 0, 55296)` emits `ed a0 80` | `drawGlyph` sentence: an Integer code point, values below `U+0020` skipped |
| overview | 2 | CONFIRMED | no example rendered | example added; `termov` on a pty drew the box and printed `drew on 80x24` |
| clear | 1 | CONFIRMED | `src/target/linux_gtk/app_io.rs:emit_app_term_clear` calls `g_idle_add(TERM_REDRAW_IDLE_SYMBOL)` | per-target present sentence |
| didResize | 1 | CONFIRMED | the GTK and macOS paths set the flag only when the cell extent changes | cell-dimensions wording |
| didResize | 2, 3 | CONFIRMED (examples broken) | `termresize` vs `termresize2`: with `term::sync` the resize is reported (`f9:100x30`), without it never (`resizes=[]`) | both examples rewritten as frame loops; the second reports `last seen 100x30` after the pty is resized |
| types | 1 | CONFIRMED | `term::TermSize` is registered read-only | `TermSize` description |
| types | 2, 3 | CONFIRMED | the fields are extents; `0 .. n-1` describes positions | `columns`, `rows` descriptions |
| drawHLine | — | NO FINDINGS | — | — |
| drawVLine | — | NO FINDINGS | — | — |
| drawGlyph | 1, 2 | CONFIRMED → **bug-725** | `termglyph` (above); `emit_draw_glyph` guards only `< 32` | description and `codepoint` parameter |
| drawGlyph | 3 | CONFIRMED | `emit_draw_glyph` stamps a `WIDE_TRAIL` at `column + 1` | double-width sentence (row 82) |
| drawText | 1 | CONFIRMED | the control-character branch tests the first byte `< 32` | `text` parameter |
| drawText | 2 | CONFIRMED → **bug-726** | `termattr` on a pty: bold on scalar 1 of decomposed `é` emits `e`, an attribute switch, then the mark in the next cell (`b"e\xcc\x81"` never appears); `toMarkdown` reports `e**́**` | overload paragraph states the split |
| drawBox | 1, 2 | CONFIRMED | `termgeom`: reversed corners drew; off-surface values clamp | `rowB`, `columnB` parameters |
| drawBox | 3 | CONFIRMED | `termgeom`: `drawBox(Double, 5, 5, 5, 5)` → one `╝`; the one-cell-wide box → `┐`, `│`, `┘` | degenerate-box sentence |
| fillRect | 1 | CONFIRMED | `termgeom`: `fillRect(Dark, 18, 2, 19, 4)` → 6 cells, so both corners are filled | inclusive-corners sentence |
| fillRect | 2, 3 | CONFIRMED | `termgeom`: `(-5, -5, -1, -1)` → 0 cells, `(-1, -1, 0, 0)` → 1 cell | `rowB`, `columnB` parameters |
| getForeground | 1 | CONFIRMED | `termnocolor`: reading `.alpha` without `IMPORT color` → `TYPE_UNKNOWN_VALUE` | import sentence |
| getForeground | 2 | CONFIRMED → **bug-727** | `emit_get_color` raises `ErrOutOfMemory`; `errors: vec![]` | — |
| getForeground | 3 | CONFIRMED (scope cut) | 24-bit packed storage | row 80 |
| getBackground | 1 | CONFIRMED | same as `getForeground` (`termnocolor`) | import sentence |
| getBackground | 2 | CONFIRMED | `termcolor`: `setForeground(rgba(0, 128, 255, 9))` reads back `0,128,255,255` | "red, green and blue match; alpha is always 255" |
| getBackground | 3 | CONFIRMED → **bug-727** | `errors: vec![]` with an `ErrOutOfMemory` raise site | — |
| getBackground | 4 | CONFIRMED (scope cut) | packed state | row 80 |
| getBold | — | NO FINDINGS | — | — |
| hideCursor | 1 | CONFIRMED (scope cut) | "retained surface", escape-sequence mechanics | row 81; `showCursor` aligned with it |
| hideCursor | 2, 3 | CONFIRMED | `src/target/linux_gtk/app_io.rs:emit_app_term_set_cursor` schedules a redraw itself | per-target present sentences on both cursor pages |
| getUnderline | — | NO FINDINGS (Fable) | reviewer probe: nine reads, off-gate returns `FALSE` with the slot still set; `term::on` resets it | — |
| isOn | 1 | CONFIRMED (Fable) | `term.rs:emit_set_cursor_visible` opens with the inactive gate; `mfb man term hideCursor` says gated | `hideCursor`/`showCursor` named in the gated list |
| isOn | 2 | CONFIRMED (Fable, scope cut) | "the module's active flag read directly" | observable half kept (row 83) |
| isOn | 3 | CONFIRMED (Fable, example weak) | the second example never calls `term::on`, so it printed nothing | rewritten as a `drawStatus` SUB called before and after `term::on` |
| moveTo | 1 | CONFIRMED (Fable) | `termwide` on a pty: `moveTo(1, 0)` then `漢C` leaves the cursor at `ESC[2;4H` — two cells for `漢` | advance-by-cell-width sentence |
| moveTo | 2 | CONFIRMED (Fable) | `termlag` on a pty resized 24x80 → 30x100 before the next `sync`: `terminalSize` → `30x100`, `moveTo(999, 999)` + `B` lands at the old last cell (`ESC[24;81H` after it) | clamp uses the surface size, refreshed at `on` and each `sync` |
| moveTo | 3 | CONFIRMED (Fable) → **bug-727** | `gen_shared.rs:lower_term_helper` wraps every member in `prepend_wrong_mode_gate(…, ModeRequirement::Console)`; `errors: vec![]` | `ErrWrongMode` sentence on `moveTo` and the overview; bug-727 gains the row |
| off | 1 | CONFIRMED (Fable) | `termofferr` on a pty: `term::on`, then an unhandled division by zero → `exit=255 entered=True restored=True`; reviewer `p_sig`: `SIGTERM` → exit 143 with the same restore sequence | "every exit path" paragraph replaced: the runtime calls `term::off` at exit |
| off | 2 | CONFIRMED (Fable) | `src/target/macos_aarch64/app/app_io.rs:emit_app_term_off_helper`: final `display`, content view swapped back to the transcript, first responder restored, line input (bug-150) | app-window paragraph after step 4 |
| setBold | 1 | CONFIRMED (Fable, scope cut) | "stored in the module's current-attribute state … no escape sequence" | "Nothing is written to the terminal by this call" (row 84); same on all four setters |
| setBold | 2 | CONFIRMED (Fable) | reviewer pty capture: `io::print` cells carry `ESC[1m` | which calls take the setting, named on all four setters |
| setBold | 3 | CONFIRMED (Fable) | `helper_astrings_bridge.rs`: `drawText` of an `AttributedString` sets each run's bold/underline and restores the saved values; colours only where a run sets one | exception sentence on all four setters (flag vs colour wording) |
| setBold | 4 | CONFIRMED (Fable) | `termgetoff`: set before `on` → `getBold` `FALSE`, `getForeground` `#ffffffff`, `getBackground` `#000000ff` | gate sentence says what the getter reports; same on all four setters |
| setUnderline | — | NO FINDINGS (Fable) | reviewer probe: nine reads | setBold's class fixes applied (identical sentences) |
| setBackground | 1 | CONFIRMED (Fable) | reviewer probe: `rgba(10, 20, 30, 128)` reads back `#0a141eff` | red/green/blue round-trip, alpha reads back 255; same on `setForeground` |
| setBackground | 2 | CONFIRMED (Fable, scope cut) | "packed into the module's current-attribute state" | as setBold 1 (row 84) |
| setBackground | — | main-thread pass | first sentence and `base` said "the colour … text will be written in" — that is the foreground | "background colour of the cells"; parameter reworded |
| setForeground | 1 | CONFIRMED (Fable) → **bug-729** | `termappfg`, `--app` on macOS: `setForeground(rgb(10, 20, 30))` reads back `#50d8df`, `setBackground(rgb(40, 50, 60))` `#70f8ff`; `app_io.rs:emit_app_set_color` packs `mfb_arg(0..2)` | — (true once fixed) |
| setForeground | 2 | CONFIRMED (Fable) | `termgetoff` and reviewer probe: white after `term::off` | "lasts for the current TUI session" on both colour setters |
| setForeground | 3 | CONFIRMED (Fable) → **bug-727** | `lower_term_helper` wrong-mode gate | `ErrWrongMode` sentence on all four setters |
| setForeground | 4 | CONFIRMED (Fable, scope cut) | as setBold 1 | row 84 |
| setForeground | 5 | CONFIRMED (Fable, example weak) | the save/restore example drew nothing | both colour setters' second example draws before and after the restore |
| on | 1 | CONFIRMED (Fable) → **bug-728** | pty + `stty -a`: `termononce` → `icanon`, `termontwice` → `-icanon` | "always guard a second `term::on`" warning (interim, until bug-728) |
| on | 2 | CONFIRMED (Fable) | reviewer `nosync`: the frame is presented by the exit-time `term::off`; `mfb man term` overview says `off` presents | "only `term::sync` presents a frame while TUI mode is on" paragraph |
| on | 3 | CONFIRMED (Fable) | `io_terminal.rs:emit_console_raw_line_mode` re-applies the raw buffer after the read | one-read sentence |
| on | 4 | CONFIRMED (Fable) → **bug-727** | `emit_on` raises `ErrOutOfMemory`; `errors: vec![]` | — (prose already names it) |
| on | 5 | CONFIRMED (Fable) → **bug-727** | as moveTo 3 | `ErrWrongMode` sentence |
| on | 6 | CONFIRMED (Fable) | reviewer `sizeon`: stdout piped → `terminalSize` raises `7-705-0007` though the surface fell back to 24x80 | fallback sentence |
| showCursor | 1, 2 | CONFIRMED (Fable) → **bug-730** | `grep -rn TV_CURSOR_VISIBLE_OFFSET src/target/macos_aarch64/` → constant, `on` store, setter store, no read; Windows the same | per-backend sentence on both cursor pages |
| sync | 1 | CONFIRMED (Fable) | reviewer pty capture: after `moveTo(0, 0)` + `io::print("hello")` the frame ends `ESC[2;1H` | cursor sentence names the last `print`/`write` |
| sync | 2 | CONFIRMED (Fable) | reviewer pty resized 24x80 → 10x40 between two syncs: the second sync emitted the full 10-row repaint | "presents this frame in full" |
| sync | 3 | CONFIRMED (Fable) | the six draw calls are buffered too (`emit_draw_line` and siblings emit no ANSI) | the buffered-call list names them |
| sync | 4 | CONFIRMED (Fable) | See also is derived from the description's references (`src/cli/man.rs:render_see_also`) | resize sentence names `term::didResize` and `term::terminalSize`, so both now appear under See also |
| terminalSize | 1 | CONFIRMED (Fable) → **bug-727** | reviewer `p_before`: `ErrUnsupported` before `term::on`; `errors: vec![]` | — (prose already names both) |
| terminalSize | 2 | CONFIRMED (Fable) → **bug-727** | `app_io.rs:emit_app_terminal_size`: the arena-alloc failure branch raises `ErrUnsupported` | macOS app sentence; bug-727 gains the row |

Fable-half probes, all in `/tmp/p125-ex/`: `termofferr`, `termwide`, `termlag`,
`termgetoff`, `termononce`, `termontwice`, `termappfg` (an `--app` build run from
its bundle); runner `/tmp/p125-tools/term-pty-run.py`.

Bugs filed from this phase (Fable half): **bug-728** (`term::on` twice leaves the
terminal in single-key mode), **bug-729** (app-mode colour setters read the wrong
arguments), **bug-730** (macOS and Windows `--app` draw no cursor); bug-727 gained
the `ErrWrongMode` and macOS `terminalSize` rows.

Bugs filed from this phase (Codex half): **bug-725** (`drawGlyph` emits invalid UTF-8),
**bug-726** (`drawText` splits a grapheme when styling changes inside it),
**bug-727** (`term` members declare no errors). Belongs-in-spec rows: 80-84.

Note: the plan-125 bugs were renumbered twice. Main took 654-657 first, so ours
moved to 658-664; by 2026-09-25 main had also taken 658, 659 and 665-720, so they
moved again to **721-727** (`git log --all --oneline | grep -oE 'bug-7[0-9][0-9]'`
→ highest 720): 721 `io::pollInput` crash, 722 `io` declared errors, 723
`io::setBuffered` drain, 724 `astrings` declared errors, 725 `term::drawGlyph`, 726
`term::drawText` grapheme split, 727 `term` declared errors.

Acceptance: 26 units `exit 0`; sweeps clean; every `term` example leaves the
terminal restored (verified by running it).
Commit: f58c14267 (Codex half, 14 of 26), 3be554816 (closed)

### Phase 4 — strings (43 units; was 42 — see Corrections 2026-10-26)

- [x] 41 pages + overview + types (`D-phase4-current.txt`).
      `--reconcile --letter D-phase4-current --units planning/plan-125-units/D-phase4-current.txt`
      → `units=43 unaccounted=0 orphans=0` (Fable, `--jobs 2`).
- [x] Every Unicode claim probe-verified; every performance hint checked
      against memory `string-concat-beats-list-join-in-mfb` before it is left
      standing. The reviewers probed each page's claims (their probes are in each
      findings file). Every claim the edits added was re-probed on the release
      binary (`/tmp/p125-ex/strprobe`): `upper("ŉ")` → `ʼN` (2 → 3 bytes), `upper("ﬁ")`
      → `FI` (3 → 2), `lower("ΟΔΟΣ")` → `οδοσ`, NFC `U+212B` → `Å` (1 scalar, 3 → 2
      bytes), `U+212A` → `K`, `U+0958` → 2 scalars, `U+F900` → `豈`, jamo → `가`;
      `lineBreaks` breaks after VT, NEL and FF (all `mandatory`), after `ab-` but not
      inside `-ing` or `-5`; `isDigit` is `TRUE` on `٣` and `३`, while `toInt("٣")` and
      `toFloat("३")` raise `77050003`; `split` on `U+0301` and `trim`/`trimChars`
      leave the mark as stated. `padRight("日本", 4, "-")` measures 6 columns with
      `term::measureText` on a pty. The Unicode 16.0 version is what
      `src/codegen/string/unicode/*_ranges.txt` pin. No strings page gives
      accumulation advice: `grep -n -i 'faster|slower|accumulat|in a loop|quadratic'`
      over `src/codegen/builtins/strings/*.rs` finds only the `stripPrefix`/`stripSuffix`
      "call it in a loop" for repeated prefixes, which is a correctness hint, not a
      performance one.
- [x] Every example compiled and run; ledger recorded.
      `MFB=$PWD/target/release/mfb ./scripts/man-run-examples.sh strings --run` →
      `examples: 88 built: 88 ran: 88 failed: 0`; `--memory-scope strings` and
      `--scope strings` → `unclassified … 0`, `internals-vocabulary hits: 0`.

#### Phase 4 ledger — Fable iteration 2 (`planning/plan-125-findings/D-phase4-current/`)

79 findings on 38 pages (`grep -c '^VERDICT:'` over the findings files); 5 pages
had none (`caseFold`, `contains`, `endsWith`, `isLetter`, `isWhitespace`). Verdicts:
40 incomplete, 26 misleading, 9 out-of-scope, 2 wrong, 1 example-broken,
1 UNVERIFIED. 78 applied. The UNVERIFIED one (`padRight` 5, six columns) needed no
edit and was then measured on a pty. The 9 out-of-scope sentences, plus the same
sentences on their siblings, were cut and logged as belongs-in-spec rows 85-92.

| Page | Findings | Disposition |
|---|---|---|
| byteLen | 1 out-of-scope | "does not walk the text" cut → row 85 |
| count | 1 | "matched literally, not as a pattern" |
| endsWithAny | 2 | `AttributedString` spelling; "like it, it never fails". The `startsWithAny` order cut and the `startsWith` scalar wording were mirrored here and on `endsWith` (not flagged there, same defect) |
| find | 3 | upper bound "greater than the scalar length"; caption matches the `TRAP` example; type spelling |
| fromScalars | 2 | the only error is `ErrOutOfMemory`; scalars vs graphemes, pointing at `graphemesCount` |
| graphemeAt, graphemes | 1 + 3 | type spelling; points at `graphemesCount`/`graphemeAt`; example now uses a decomposed `é` and says it prints five lines |
| graphemesCount | 1 | fallibility difference from `graphemes` stated |
| isDigit, isLower, isUpper | 2 + 1 + 1 | Unicode 16.0; `isDigit` vs `toInt`/`toFloat` (probed) |
| join | 1 | the `ErrOutOfMemory` condition |
| left, right | 1 + 1 | `count` zero/negative outcomes |
| lineBreaks | 4 | VT and NEL added; hyphen rule conditioned; trailing spaces belong to the line before, the example measures `trimEnd`; types `index`/`mandatory` descriptions |
| lower, upper | 3 + 3 | final sigma; type spelling; unconditional attribute drop; `ŉ`/`ﬁ` byte examples; the OOM sentence moved out of the AttributedString paragraph |
| mid | 1 → code | descriptor declares `ErrOutOfMemory`; the new `intrinsic_declared_errors` guard (56c0406c5) found and fixed `strings::replace`, `collections::mid` and both `utf8` codecs the same way |
| normalizeNfc | 4 + 1 out-of-scope | singletons and exclusions; length can grow; the OOM condition; Unicode 16.0; Hangul mechanism cut → row 86 |
| overview | 3 | negative-argument errors split between `ErrInvalidArgument` and `mid`'s `ErrIndexOutOfRange`; `mid` negative case; `lineBreaks` returns `LineBreak` records |
| padLeft, padRight | 4 + 1 out-of-scope, 4 + 1 UNVERIFIED | huge-width OOM vs `ErrInvalidArgument`; one scalar is not one character; "No argument"; the pair named; unstyled padding; identity sentence cut → row 90 |
| padLeftToWidth, padRightToWidth | 1 + 1 | control characters count as one column |
| repeat | 3 | both size errors; every copy carries the styling |
| replace | 2 | inserted text unstyled; empty `value` |
| split | 3 | grapheme split by a combining-mark delimiter; the OOM condition; alias shown as call syntax |
| startsWith | 3 | can only *end* inside a character; one scalar-by-scalar wording; `prefix` is a plain `String` |
| startsWithAny | 1 out-of-scope | short-circuit cut → row 87 |
| stripPrefix, stripSuffix | 3 + 1, 1 out-of-scope each | "never fails" replaced with the OOM-only statement; span handling at the cut; identity sentence cut → row 91 |
| toBytes, toScalars | 1 out-of-scope + 1 + 1 out-of-scope | storage/walk wording cut → rows 88-89; the OOM condition |
| trim, trimStart, trimEnd, trimChars | 1 + 1 + 1 out-of-scope + 1 | `AttributedString` in the parameter row; scalars not graphemes; identity sentence cut → row 92 |
| types | 2 | "never 0" moved onto `lineBreaks`; the last entry's index is the text length |

Fallout fixed alongside: the `AttributedString` rewording broke `inplace_self_update_census`'s Tier-B sentence on `padLeft`/`padRight`, so those pages keep the census sentence ("it returns an `AttributedString`"). The census is green.

Acceptance: 43 units `exit 0` (`--reconcile --letter D-phase4-current`); sweeps clean; no page advises an accumulation
pattern the measurement contradicts.
Commit: —

### Phase 5 — fs (43 units)

- [ ] 41 pages + overview + types page.
- [ ] **Every example rewritten to temp paths and run**; the ledger records
      the temp path per example and confirms nothing is left behind.
- [ ] Handle contract wording verified by reading, not grepping
      (`--memory-scope fs` is necessary and not sufficient).
- [ ] Ledger recorded.

Acceptance: 43 units `exit 0`; `--reconcile` exits 0 over the whole 144-unit
batch; the example ledger accounts for every example, 0 unaccounted; no
`fs` example references a path outside a temp directory
(`./scripts/man-run-examples.sh fs --run` clean, and a read-through of the 41
examples confirming it).
Commit: —

### Phase 6 — pages main changed after they were reviewed (39 units; was 11 — see below)

Added 2026-09-25. `main` was merged into this branch mid-letter (`a24a3894b`, 812
commits); it had changed the lowering or prose under pages Phases 1-3 had already
closed, and added two `term` members. Units:
`planning/plan-125-units/D-phase6.txt` — `io` `input`, `pollInput`, `readByte`,
`readChar`, `readLine`, `setBuffered` (mouse-aware reads, plan-94-B, bug-669);
`term` `overview`, `types`, `isOn`, and the new `enableMouse` and `pollMouse`.

Re-derived 2026-10-26 after the fast-forward to `d58b5397a`. Baseline: the release
`mfb` built at Phase 3's close `3be554816` (no main merge falls between Phases 1-3's
closes: `git log --merges --first-parent 6f6c05547..3be554816` is empty), against
the release `mfb` at HEAD. Each page of `mfb man <pkg> --all` was diffed after
normalizing the language-wide mechanical changes (`SUB` → `FUNC … AS Nothing`,
dropped `RES` parameter keywords, the `AttributedString` spelling) and dropping the
derived signature lines and error tables; a page counts when any authored line
differs (scripts: `/tmp/p125-cmp/split.py`). Units:
`planning/plan-125-units/D-phase6-current.txt` — `io` 9 (overview, input, readLine,
setBuffered; new readGrapheme, readKey, readRaw, readScalar, types — pollInput,
readByte and readChar no longer exist), `astrings` 6 (overview, addAttribute,
clearAttributes, getAttributes, removeAttribute, toMarkdown), `term` 24 (21 changed
plus the new enableMouse, measureText, pollMouse). The first list's 11 units are all
in it or gone from the package.

- [ ] 39 units reviewed and applied; ledger recorded.
- [ ] Every example on the 11 pages compiled and run (`term` and mouse ones on a
      pty), terminal restored.
- [ ] `--memory-scope` and `--scope` clean for `io` and `term`.

Acceptance: `--reconcile --letter D-phase6-current` → 39 units, 0 unaccounted; sweeps
clean; examples run.
Commit: —

## Validation Plan

- Tests: none (man prose); pinned-text tests updated in the same commit if
  touched.
- Coverage check: `--reconcile` over the 144-unit list; example ledger
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

- **2026-09-25 (owner): the reviewer is Claude Fable 5.1, not Codex.** Every review
  unit now runs `claude -p --model claude-fable-5-1` through
  `scripts/doc-review-fanout.sh` (manifest banner `Claude Code v…/claude-fable-5-1`);
  the Codex plan's usage limit stalled plan-125-D Phase 3 twice. This reverses
  plan-125-A's rejection of a Claude reviewer ("the value is independence from
  Claude"): the reviewer is now a different model from the main thread, not a
  different vendor. The Fable reviewer also has no OS sandbox (Codex's
  `workspace-write` did); its `git commit`/`git push` are denied and the harness's
  DIRTY check still guards the tree. The prompts are unchanged.
- **2026-09-25: `strings` is 42 units, not 40, so the batch is 144.**
  `./scripts/man-census.sh --fill strings` → `PAGES 41` (`D-phase4.txt` already
  listed 42 lines). No other letter's scope was derived from the 40.
- **2026-09-25: the harness reviewed in a gutted worktree.** The first Phase 3
  resume ran in `/tmp/plan-125-worktrees/D-phase3-1`, which macOS's periodic `/tmp`
  sweep had stripped of its `.git` file and most of `src/`; the other three were
  intact but still sat at the 2026-09-19 base, so a resume would have reviewed
  stale prose. `doc-review-fanout.sh` reused any existing worktree unchecked. Fixed
  in the harness: a reused worktree must be a sound worktree (else it is recreated)
  and is forced to the run's base. The one row produced in the gutted worktree
  (`term/isOn`, first Fable row) was re-run; `--reconcile` reads the last row. The
  same sweep deleted the Phase 3 pty runner and probes under `/tmp/p125-ex`; the
  runner was rebuilt under `/tmp/p125-tools/`.
- **2026-09-25: main merged mid-letter (`a24a3894b`).** Main was 812 commits
  ahead, with 99 changed files under D's five packages (`git diff --stat
  HEAD...main -- src/codegen/builtins/{io,astrings,term,strings,fs}`): bug-694 gave
  every `fs` member its declared errors, plan-94-B added mouse input (`term`
  `enableMouse`/`pollMouse`, mouse-aware `io` reads), and several `strings`
  lowerings changed. The prerequisite "this branch's pages are current" had gone
  false, so reviewing on without merging would have applied findings to stale
  pages. Consequences: the 9 `fs` reviews already back (stale base) were discarded
  and all 43 re-run on the merged tree; 22 `strings` units whose lowering or prose
  main changed were re-run (`D-phase4-postmerge.txt`); the closed `io`/`term` pages
  main touched get Phase 6. `term` is now 26 pages + overview + types = 28 units.
  Conflicts: `term/mod.rs` (main's `MouseEvent` paragraph kept, this letter's
  overview example after it) and `collections/append.rs` (main's corrected cost
  rule kept, plus this branch's `TYPE_ASSIGN_REQUIRES_MUT` sentence).
- **2026-09-25: bugs 658-664 renumbered to 721-727** — main had taken 658, 659 and
  665-720 meanwhile (`git log --all --oneline | grep -oE 'bug-7[0-9][0-9]'` →
  highest 720). New bugs from here continue at 728.
- **2026-09-26: bug-721 renumbered to bug-742** before the second merge of main:
  main had taken 721 for an unrelated bug (Windows arena free list) and 732-737
  meanwhile (`git ls-tree -r --name-only main bugs | grep -oE 'bug-7[2-9][0-9]'`).
  It was first moved to 738, but 738-741 were already taken on unmerged branches
  (`git log --all --oneline | grep -oE 'bug-7[3-9][0-9]'`: 738 on
  `worktree-B-riscv-ret`, 739 `worktree-B-736`, 741 `worktree-B-735`; 740 is an
  uncommitted doc in worktree `737`) — checking `main` alone missed them. The
  `io::pollInput` exit-crash bug is now **bug-742**; 722-731 appear on no branch,
  in no worktree, and in no `completed/` directory, so they did not collide. New
  plan-125 bugs must check all refs and every worktree's `bugs/`, not just `main`.
  The historical 721 references above describe the first renumbering.

- **2026-10-26: the branch was 2589 commits behind main; fast-forwarded to
  `d58b5397a`.** `git rev-list --count HEAD..main` → 2589, `main..HEAD` → 0 (every
  plan-125 commit was already on main), so `git merge --ff-only main` was clean.
  Consequences, all measured with the release `mfb` at HEAD:
  - The `D-phase4-postmerge`, `D-phase5` and `D-phase6` runs never reviewed most of
    their units: 17 of 22, 39 of 43 and 8 of 11 rows are `FAILED` after 2-5 s, and
    the logs say `api_error_status":429 … "You've hit your session limit"`
    (`planning/plan-125-findings/D-phase4/man-page-strings-right.log`). The
    findings directories for those three runs were never committed.
  - `strings` is now **43 units**, not 42: `displayWidth` was removed
    (`4e2ed201f refactor(strings): remove display width API`), `lineBreaks` was
    added (`c4a3bf5c4 plan-162-E`), and a `types` page renders
    (`mfb man strings --all`, page titles before each `═` rule). 22 non-plan-125
    commits changed `src/codegen/builtins/strings` since `c420fec6c`
    (`git log --no-merges c420fec6c..HEAD -- src/codegen/builtins/strings | grep -vc plan-125`).
    Phase 4 is therefore re-run whole on the current tree as
    `planning/plan-125-units/D-phase4-current.txt` (43 units); the older
    `D-phase4` findings stay as the record of the first pass.
  - `fs` is now **47 units** (45 functions + overview + types), not 43; Phase 5
    runs `planning/plan-125-units/D-phase5-current.txt`, cross-checked against the
    rendered page titles of `mfb man fs --all`.
  - Phases 1-3 (`io`, `astrings`, `term`) were closed before 2589 more commits of
    main landed; Phase 6's 11-unit list was derived from the first merge only and
    is re-derived from a fresh diff (see Phase 6).

## Summary

Two risks dominate: an `fs` example a developer runs against a real path, and
a Unicode claim in `strings`/`astrings` that is true by reasoning and false in
this binary. Both are held by *running the thing* rather than reading it, which
is the only reason iteration 2 costs what it costs.
