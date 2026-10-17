# plan-176-F: MFBASIC sources (acceptance suites, package tests, fixtures, money-demo)

Last updated: 2026-10-17
Effort: large (3h–1d)
Depends on: plan-176-E

F splits the 22 over-budget `.mfb` files: 15 suite files of the `tests/acceptance` project,
three package test files (`packages/xml`, `packages/jwt`), three `tests/rt-behavior` fixture
programs, and the one over-budget MFBASIC code file, `examples/money-demo/src/main.mfb`. Every
project involved globs `**/*.mfb` from `src`, so a split is the motion of whole top-level
declarations and whole `TGROUP`s into new sibling files. No TCASE is added, removed or renamed,
and no program changes behavior. One rt-behavior fixture,
`tests/rt-behavior/lexical/literal-spans-lines-valid/src/main.mfb`, is not split; it gets an
`exempt` row instead.

The single outcome: after F lands, `cargo test --test file_size_budget` shows no row for any of
F's files, and `tests/guards/file_size_ceilings.txt` has no `ceiling` row for any of them. Its
only F entry is the `exempt` row for `literal-spans-lines-valid/src/main.mfb`.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule, the MFBASIC counting rules (§2), and
  §Decisions.
- `.ai/testing-gates.md`: line 18 (`tests/acceptance` has no `golden/`, so the harness runs
  `mfb test` and compares nothing), line 223 (`tests/acceptance` is ONE project and top-level
  FUNC names are shared), and § "Acceptance golden harness mechanics" (`test-accept.sh`,
  `sync-goldens.sh`, the fixture count is a signal).
- `.ai/resources-packages.md`: package projects and their test files.
- Spec topics that set the motion rules:
  - `src/docs/spec/language/04_types.md` and `06_functions.md`: `PRIVATE` is file-local, and
    IMPORTs are per file.
  - `05_bindings-and-scope.md`: PUBLIC is visible anywhere in the project.
  - `22_test-framework.md` § Structure: TESTING blocks may appear in any file, any number of
    times, and run in declaration order across files.
- `src/codegen/builtins/tests/math.rs:4` cites `tests/acceptance/src/arithmetic.mfb:203` (stays exact).
- `packages/xml/oracle/README.md:164` names `test_xpath_edges.mfb` (stays correct).
- `tests/cli/cli_fmt_multiline_brackets.rs:22` reads `literal-spans-lines-valid/src/main.mfb`
  by path (the file is not split).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-E is complete and archived | `ls planning/plan-176-E-* planning/completed/plan-176-E-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14) |
| F's files have no uncommitted changes | `git status --porcelain -- examples/money-demo packages/jwt/src packages/xml/src tests/acceptance tests/rt-behavior/datetime/datetime-arith-errors-rt tests/rt-behavior/types/union-variant-expression-wrap tests/rt-behavior/lexical/literal-spans-lines-valid` → empty | MET (2026-10-17) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you. The reader needs to know how far off the gate is,
> not just which row you hit first.

## 1. Goal

- Every F test file and every file F creates is ≤ 250 test LOC, and `money-demo`'s files are
  each ≤ 1000 code LOC (`python3 -I tools/loc-census/census.py --file <path>`). The exception
  is the exempt `literal-spans-lines-valid/src/main.mfb` (523).
- `cargo test --test file_size_budget` prints no row naming an F path, and the ceiling file
  holds no `ceiling` row for one.
- `mfb test` reports the same `Tests: N  Pass: N  Fail: 0` for `tests/acceptance`,
  `packages/xml` and `packages/jwt` before and after.
- The split rt-behavior fixtures print byte-identical output: `build.log` and `.run` goldens
  are unchanged.

### Non-goals (explicit constraints)

- No behavior change. No TCASE is renamed, added or removed, and no TCASE body is edited.
- The edits beyond moving whole declarations are listed in §4:
  - copying IMPORT blocks;
  - re-opening the wrapper `TGROUP` in each new file;
  - dropping `PRIVATE` from 9 test helpers (5 in `jwt`, 4 in `xml/test_write`);
  - one verbatim duplicate of the 3-line `PRIVATE FUNC catalog` in `xml`.
- No `golden/` directory is added to `tests/acceptance` (`.ai/testing-gates.md:18`: its
  presence is a mode switch).
- No golden other than the `.ast` and `.ir` of the two split rt-behavior fixtures changes.
- No package's public surface changes: only test files and test-file helpers are touched in
  `packages/`.

## 2. Current State

The LOC figures come from the planning census (`python3 -I /tmp/loc/census.py "$PWD"`, working
tree of `17a9b5799`) and `python3 -I /tmp/loc/span.py <file>`. The one file that changed since,
`tests/acceptance/src/astrings.mfb`, was re-measured on 2026-10-17 at `a3c95fc07`. TCASE counts
come from `grep -c '^ *TCASE ' <file>`.

| File | Kind | LOC | TCASEs |
|---|---|---|---|
| `examples/money-demo/src/main.mfb` | code | 1417 | 0 |
| `packages/jwt/src/test_verify.mfb` | test file | 252 | 36 |
| `packages/xml/src/test_write.mfb` | test file | 281 | 43 |
| `packages/xml/src/test_xpath_edges.mfb` | test file | 255 | 44 |
| `tests/acceptance/src/arithmetic.mfb` | test file | 368 | 40 |
| `tests/acceptance/src/astrings.mfb` | test file | 680 (682 at `17a9b5799`; `2e87cc224` removed 2 lines from "Tier-A queries answer on the visible text") | 71 |
| `tests/acceptance/src/bits.mfb` | test file | 306 | 55 |
| `tests/acceptance/src/collections.mfb` | test file | 512 | 35 |
| `tests/acceptance/src/crypto.mfb` | test file | 774 | 91 |
| `tests/acceptance/src/csv.mfb` | test file | 257 | 32 |
| `tests/acceptance/src/encoding.mfb` | test file | 268 | 41 |
| `tests/acceptance/src/general.mfb` | test file | 343 | 22 |
| `tests/acceptance/src/json.mfb` | test file | 481 | 54 |
| `tests/acceptance/src/math.mfb` | test file | 588 | 55 |
| `tests/acceptance/src/money.mfb` | test file | 251 | 28 |
| `tests/acceptance/src/primitives.mfb` | test file | 326 | 34 |
| `tests/acceptance/src/regex.mfb` | test file | 578 | 71 |
| `tests/acceptance/src/strings.mfb` | test file | 286 | 36 |
| `tests/acceptance/src/vector.mfb` | test file | 749 | 71 |
| `tests/rt-behavior/datetime/datetime-arith-errors-rt/src/main.mfb` | test file | 275 | 0 |
| `tests/rt-behavior/lexical/literal-spans-lines-valid/src/main.mfb` | test file | 523 | 0 |
| `tests/rt-behavior/types/union-variant-expression-wrap/src/main.mfb` | test file | 275 | 0 |

`mfb test` baselines, measured 2026-10-17 with `target/release/mfb` built 2026-10-14 14:29,
`target/release/mfb test <project> | grep '^Tests:'`:

| Project | Result | Time |
|---|---|---|
| `packages/xml` | `Tests: 253  Pass: 253  Fail: 0` | 14.5 s |
| `packages/jwt` | `Tests: 102  Pass: 102  Fail: 0` | 28.5 s |
| `tests/acceptance` | `Tests: 785  Pass: 785  Fail: 0` | 132 s |

The 15 acceptance files hold 736 of those 785 TCASEs (sum of the table). F re-takes the
baseline with the HEAD release binary at F0 (§Phases), because the gate compares F's tree
against itself.

### Verified properties

- **Every affected project globs `**/*.mfb` from `src`.** Read with
  `cat tests/acceptance/project.json packages/xml/project.json packages/jwt/project.json examples/money-demo/project.json`
  and the three fixture `project.json` files. A new sibling file is compiled with no
  manifest edit.
- **Top-level PUBLIC declarations are project-visible, `PRIVATE` is file-local, and IMPORTs
  are per file** (spec topics above). So every new file copies the original file's whole
  IMPORT block. There is no unused-import diagnostic
  (`grep -in "unused.*import" src/docs/spec/diagnostics/01_rule-codes.md` → none).
- **TESTING blocks and TGROUP names may repeat across files.** Duplicate `TGROUP` names have
  no rule code (`grep -n TGROUP src/docs/spec/diagnostics/01_rule-codes.md` → only
  1-102-0010/0011/0012). The `mfb test` report shows a re-opened top group once per file.
  `tests/acceptance` and the packages have no `golden/` to compare it against.
- **`test_*.mfb` is a naming convention, not a discovery rule.**
  `grep -rn 'starts_with("test_' src repository --include='*.rs'` → none. The file-size guard
  classifies `test_*.mfb` as test (plan-176-A §2), so new package test files keep the prefix.
- **Splitting an MFBASIC file changes `.ncode`, so byte-identity is not a gate for F.** Probe
  `/tmp/loc_f_probe`: the same program in one file and in two files, built with
  `mfb build -q --ncode`. `shasum` differed. The diff (75 lines) is source-path/line string
  constants (`"src/helpers.mfb"`) and the string-table renumbering they cause. Behavior is
  therefore the gate: `mfb test` counts, `.run`/`build.log` goldens, and the money-demo run.
- **`money-demo` runs headless under a pty.** Probe `/tmp/loc_f_md` (a copy of the project):
  `mfb build -q .` (6.5 s), then
  `(sleep 3; printf q) | script -q ts sh -c 'stty rows 40 cols 120; ./build/money-demo.out'`
  exited 0, and `grep -a -o 'Welcome. Starting balances' ts` matched. At 80×24 it shows the
  "needs at least 80x25" screen instead, so the check pins 120×40.
- **The rt-behavior goldens.** Listed with `ls <fixture>/golden`:
  - `datetime-arith-errors-rt` has `build.log`, `.ast`, `.ir`, `.run`. Its `.run` contains no
    file or line (`grep -c '\.mfb\|line' …run` → 0).
  - `union-variant-expression-wrap` has the same four. Its `.run` is empty (0 lines), and the
    program output lives in `build.log`.
  - The `.ast` records `"files": [{ "path": "src/main.mfb", … }]` and per-item `"line"`.
  - The `.ir` records per-item `"line"`. Its only file-path string is an `ErrorLoc` in
    `mkf`, at line 50 of `union-variant-expression-wrap`. That function stays in `main.mfb`
    (`grep -n '"file"\|main.mfb' …ir`).
- **The `arithmetic.mfb:203` citation stays exact.** `src/codegen/builtins/tests/math.rs:4`
  cites `tests/acceptance/src/arithmetic.mfb:203` (`expectFloat(fa MOD fn, …)` in "modulo"). No
  line above 274 leaves `arithmetic.mfb`.
- **No spec citation, guard, script or `.ai` path names an F file other than those listed.**
  `grep -rnoE "\[\[(tests/acceptance|packages/(xml|jwt)|examples/money-demo)[^]]*\]\]" src/docs/spec`
  → none. `grep -rln "money-demo/src\|test_verify.mfb\|test_write.mfb" tests/guards scripts .ai src/docs .github`
  → none (the one `test_write` substring is the Rust fn
  `build_project_coverage_test_writes_a_report`). `grep -rn union-variant-expression-wrap tests/guards scripts .ai src/docs src tests`
  over `.rs`/`.sh`/`.md` → none. `src/codegen/builtins/datetime/mod.rs:981` names
  `datetime-arith-errors-rt` without a path.
- **Formatting baseline.** Running `mfb fmt --check <file>` over the 22 F files, two fail
  today: `tests/acceptance/src/money.mfb` and `literal-spans-lines-valid/src/main.mfb`.
  - `money.mfb` has 10 lowercase `true` keywords. A `/tmp/loc_f_fmt` copy formatted with
    `mfb fmt` differed only in `true` → `TRUE`. F fixes this in Phase 3.
  - `literal-spans-lines-valid` is not edited by F. `tests/cli/cli_fmt_multiline_brackets.rs`
    formats a copy of it on purpose.
- **`build/` is ignored.** `git check-ignore -v examples/money-demo/build` → `.gitignore:24:build/`,
  so `mfb build`/`mfb test` in place leave the tree clean.

## 3. Design Overview

Conventions (all files):

- A new file carries the original file's whole `IMPORT` block, verbatim.
- The header (everything before `TESTING`: imports, banner, helper FUNCs) stays in the
  original file. Helpers are default-visibility (PUBLIC), so moved groups still reach them.
  The exceptions are the PRIVATE package helpers in §4.2.
- An acceptance file that wraps its groups in one top `TGROUP "<name>"` gives each new file
  `TESTING` / `TGROUP "<name>"` / its groups / `END TGROUP` / `END TESTING`, a 4-line overhead.
  Without a wrapper, the overhead is 2 lines.
- Groups move whole, in their original relative order within the file that receives them.
- New file names: acceptance `<suite>_<topic>.mfb`, package `test_<area>_<topic>.mfb`, fixture
  `<topic>.mfb`.

The gate class is behavior. `mfb test` totals for the three test projects, byte-identical
`.run` and `build.log` for the fixtures, and a build-and-run of `money-demo` on every
`build-examples.sh` target. Byte-identity is not available (§2 probe).

Risk concentrates in the two split rt-behavior fixtures, the only F files with goldens. Their
`.ast`/`.ir` change by design: they record the file list and lines. Per AGENTS.md, that
re-baseline happens only after the full acceptance suite runs, and only for those two
fixtures. It is proven by a location-stripped diff. If `.run` or `build.log` moves, that is a
bug (a cross-file union-wrap or global-initialisation fault) to hunt, never to re-baseline.
These fixtures land last.

Rejected alternatives:

- *Making `xml`'s `catalog()` PUBLIC.* `packages/xml/src/test_xpath_eval.mfb:18` declares a
  different `PRIVATE FUNC catalog() AS Document` with the identical signature, which would
  collide in that file's overload set. The 3-line helper is duplicated PRIVATE instead.
- *Splitting `literal-spans-lines-valid`.* `FUNC main` (52-554 as of 17a9b5799) is 484 LOC
  (`span.py … 52 554`). Two of its statements are single list literals of 181 LOC
  (`LET testD`, 90-270) and 183 LOC (`LET testE`, 271-454). Those literals are the fixture's
  subject (bug-821: leading-comma and trailing-comma+comment forms, compared to their one-line
  twin inside `main`). A statement cannot be split. Moving parts of `main` into new FUNCs is a
  logic rewrite with new IR, not motion. `tests/cli/cli_fmt_multiline_brackets.rs:22` also
  runs `mfb fmt` over that exact path.
- *Fewer, larger money-demo files.* Two files would reach 1000, but the five-way split follows
  the program's responsibilities (state/simulation, formatting, ledgers, reports, screen) and
  costs nothing extra.

## 4. Detailed Design

The LOC figures come from the map's `span.py` sums over the listed line ranges, plus header,
import and wrapper lines (calculator `/tmp/loc/work/plan_f.py` + `plan_f.json`). Line ranges
are as of 17a9b5799, and group and FUNC NAMES are authoritative if lines drift. Test LOC for
every file under `tests/` and every `test_*.mfb` counts the whole file, helpers included.

### 4.1 `examples/money-demo/src/main.mfb` (code 1417)

All files are in `examples/money-demo/src/`. Every new file begins with the original 9-line
IMPORT block (lines 1-9: collections, color, datetime, errorCode, io, math, money, strings,
term). With those 9 lines, every file stays ≤ 427 code LOC.

- `main.mfb` (418): the IMPORT block, the banner, `ENUM Kind`, `ENUM Acct`, all `TYPE`s (`Event` …
  `Row`), every top-level `LET` constant (138-210), the simulation-state `MUT`s (1121-1139),
  `startDate`, `centsToMoney`, `pushLog`, `mk`, `eventsFor`, `record`, `simulateDay`
  (1141-1299), and `main` (1530-1650). 149 + 159 + 110.
- `format.mfb` (134 + 9): `groupDigits`, `withCommas`, `dollars`, `signed`, `fm`, `fx`,
  `fmPlain`, `ff`, `ffPlain`, `fraw`, `fsci`, `thousandths`, `bpText`, `fit`, `fitRight`,
  `modeName`, `line`, `pct`, `cents`, `moneyMax`, `incomeTax` (211-375).
- `ledger.mfb` (406 + 9): the Money ledger `openingMoney` … `dueText` (376-581), the Float
  ledger `fpct` … `postFloat` (582-816), and `replayMoney`, `replayFloat` (817-838).
- `report.mfb` (266 + 9): `sumMoney`, `sumFloat`, `stampOf`, `isHalfCent`, `notBinary`,
  `balanceRow`, `roundingReport`, `driftRow`, `floatReport` (839-1120).
- `screen.mfb` (209 + 9): `toneColor`, `put`, `isMove`, `entryTone`, `fEntryTone`,
  `recentRows`, `expenseRows`, `drawColumn`, `moneyOrFloat`, `drawFrame`, `drawModal`,
  `nextKey` (1300-1529).
- Surface: none. The file has no `PRIVATE` (`grep -n PRIVATE examples/money-demo/src/main.mfb`
  → none). All top-level `LET`/`MUT` stay together in `main.mfb`, so no global initializer
  reads another file's global. `MUT mBook = openingMoney()` and `fBook = openingFloat()` call
  into `ledger.mfb`; they are function calls, not global reads.
- Hazards: none. There is no spec citation, no path reference, and the README names no source file.

### 4.2 Package test files

**`packages/jwt/src/test_verify.mfb`** (252, 36 TCASEs)
- `test_verify.mfb` (154): header 1-44 (5 imports, plus helpers `macKeyFor`, `pinnedOptions`,
  `craftSigned`, `simpleClaims`, `withSegment`) + groups "token shape" (46-84),
  "Base64url canonicality" (86-135), "algorithm" (137-183), "duplicate members" (185-214).
- `test_verify_keys.mfb` (105): IMPORT json/collections/strings/encoding/crypto + `TESTING` +
  "signature" (216-248), "key selection" (250-287), "header parameters" (289-316),
  "unverified reads" (318-332).
- Surface: the 5 helpers are `PRIVATE`, and both halves call all of them. `PRIVATE` is dropped
  on the 5 (5 edits). There is no collision: `grep -ln "FUNC <name>\b" packages/jwt/src/*.mfb`
  finds each only in `test_verify.mfb`.

**`packages/xml/src/test_write.mfb`** (281, 43 TCASEs)
- `test_write.mfb` (178): header 1-73 (imports collections/strings; helpers `fixtures`,
  `noAttrs`, `noKids`, `rootNode`) + "compact output" (75-101), "escaping" (103-142),
  "refusals" (144-215), "content is preserved" (217-239).
- `test_write_pretty.mfb` (107): IMPORT collections/strings + "pretty output" (241-307),
  "indent clamping" (309-331), "pretty output preserves content" (333-349),
  "the content projection itself" (351-376).
- Surface: the moved half calls all four helpers (`fixtures` at 311/335/343, `noAttrs` and
  `noKids` at 281, `rootNode` at 244-329). `PRIVATE` is dropped on the four (4 edits). There
  is no collision: `grep -lnE "FUNC (fixtures|noAttrs|noKids|rootNode)\b" packages/xml/src/*.mfb`
  → `test_write.mfb` only.

**`packages/xml/src/test_xpath_edges.mfb`** (255, 44 TCASEs)
- `test_xpath_edges.mfb` (200): header 1-21 (imports, `PRIVATE FUNC catalog`) +
  "arithmetic over the special values" (23-58), "comparisons over the special values" (60-83),
  "the arithmetic branches a coverage run named" (85-120),
  "the comparison rule, in every combination" (122-183), "the document node" (185-209),
  "attributes" (211-247), "filters and short circuits" (249-262).
- `test_xpath_edges_refusals.mfb` (62): IMPORT collections/strings + a verbatim copy of
  `PRIVATE FUNC catalog() AS Document … END FUNC` (lines 18-20) + "argument errors" (264-293),
  "the parser's remaining refusals" (295-331).
- The duplicated `catalog` is the one non-motion edit in F, and the commit message names it.
- `packages/xml/oracle/README.md:164` names this file for the document-node case. That group
  stays here.

### 4.3 tests/acceptance (one project; no `PRIVATE` anywhere: `grep -n PRIVATE tests/acceptance/src/*.mfb` → none)

The new files are in `tests/acceptance/src/`. Every wrapper group named below is re-opened in
each new file.

- **`arithmetic.mfb`** (368, 40) — wrapper `TGROUP "General Arithmetic"`.
  - `arithmetic.mfb` (217): header 1-91 (helpers `byteAdd` … `moneyDivM`) + "add and
    subtract", "multiplication", "division", "modulo", "exponentiation", "type promotion",
    "precedence" (98-269).
  - `arithmetic_errors.mfb` (103): "byte errors" (274-286), "integer errors" (291-311),
    "fixed errors" (316-323), "float errors" (328-344), "float finiteness boundary" (350-373),
    "conversion boundaries" (440-453).
  - `arithmetic_money.mfb` (60): "money arithmetic" (378-419), "money errors" (424-435).
- **`astrings.mfb`** (680, 71) — wrapper `TGROUP "Builtin: astrings"`.
  - `astrings.mfb` (218): header 1-141 + "fromString / toString / defaults" (145-164),
    "constructors" (166-212), "error surface" (384-436).
  - `astrings_attributes.mfb` (175): "addAttribute / getAttributes" (214-285),
    "removeAttribute" (287-349), "clearAttributes" (351-382).
  - `astrings_spans.mfb` (193; 195 before `2e87cc224`): "Tier-A queries answer on the visible
    text" (438-485), "Tier-B transforms remap spans" (487-629).
  - `astrings_render.mfb` (124): "concatenation" (631-665), "toMarkdown" (667-725),
    "value semantics and io seams" (727-749).
- **`bits.mfb`** (306, 55) — wrapper `TGROUP "Builtin: bits"`. The bare TCASEs directly under
  the wrapper (118-125, 194-201, 204-217) stay in order in `bits.mfb`.
  - `bits.mfb` (204): header + band, bor, bxor, bnot, sl, TCASE
    "sl, sr, sra - security…", sr, sra, rl32, TCASE "rr32 - high 32 bits…", TCASE
    "rr32, rl64, rr64 - negative…", rr32, rl64, rr64 (24-255).
  - `bits_swap_count.mfb` (108): bswap16, bswap32, bswap64, clz, ctz, popCount, identities (258-371).
- **`collections.mfb`** (512, 35) — wrapper `TGROUP "Builtin: collections"`. Regrouped by
  responsibility. The report order changes, and there is no golden.
  - `collections.mfb` (190): header 1-66 + the updates: "append" (69-103), "insert"
    (223-240), "prepend" (287-299), "removeAt" (318-330), "removeKey" (332-344), "replace"
    (346-371), "set map" (373-384), "set" (386-398).
  - `collections_queries.mfb` (215): "contains" (105-120), "filter" (122-143), "find"
    (145-163), "forEach" (165-172), "getOr" (174-196), "get" (198-210), "hasKey" (212-221),
    "keys" (242-255), "len" (257-268), "mid" (270-285), "reduce" (301-316), "sum" (400-411),
    "transform" (413-430), "values" (432-445).
  - `collections_helpers.mfb` (117): "helpers" (447-524), "list bindings" (526-543),
    "map bindings" (545-562).
- **`crypto.mfb`** (774, 91) — wrapper `TGROUP "Builtin: crypto"`.
  - `crypto.mfb` (205): header 1-45 + SHA-2 (50-83), SHA-3 (86-102), SHA-1 (105-113),
    shake256 (116-133), hmac (136-170), hkdf (173-209), pbkdf2 (212-239).
  - `crypto_encrypt.mfb` (189): "seal / open — AEAD" (242-317), "encrypt / decrypt — RFC 9180
    HPKE (base mode)" (564-670).
  - `crypto_keys.mfb` (126): "generate — key pairs" (320-361), "convert — key encoding"
    (479-507), "exchange — X25519 / X448 …" (510-561).
  - `crypto_sign.mfb` (114): "sign / verify" (364-476).
  - `crypto_random.mfb` (98): randomBytes (673-687), randomInt (690-706), uuid4 (709-722),
    uuid7 (725-740), ulid (743-752), constantTimeEqual (755-771).
  - `crypto_flows.mfb` (87): "flows — end-to-end" (774-855).
- **`csv.mfb`** (257, 32) — wrapper `TGROUP "Builtin: csv"`.
  - `csv.mfb` (178): header 1-68 + "parse" (73-172), "stringify" (175-211).
  - `csv_read.mfb` (88): "readRow + parse(File)" (214-292). It copies all 5 imports, including `fs`.
- **`encoding.mfb`** (268, 41) — wrapper `TGROUP "Builtin: encoding"`.
  - `encoding.mfb` (168): header + base32 … varint (49-195).
  - `encoding_charsets.mfb` (107): utf8 (197-212), utf16 (214-231), utf32 (233-250),
    codepage (251-307).
- **`general.mfb`** (343, 22) — no wrapper group (the groups sit directly in TESTING, a 2-line
  overhead). `IMPORT errorCode` is at line 36, not in the top block, so every new file copies
  all three imports (strings, collections, errorCode).
  - `general.mfb` (210): header 1-144 + "len" (146-171), "toString" (173-206), "toInt" (208-249).
  - `general_numeric.mfb` (83): "toByte" (251-257), "toFixed" (259-278), "toFloat" (280-289),
    "typeName" (291-302), "isNumeric" (304-320), "decimalExponent" (322-333).
  - `general_strings_errors.mfb` (60): "find" (335-344), "mid" (346-357), "replace" (359-375),
    "error" (377-395).
- **`json.mfb`** (481, 54) — wrapper `TGROUP "Builtin: json"`.
  - `json.mfb` (209): header 1-170 + "parse — scalars and numbers" (175-204),
    "parse — strings, escapes, Unicode" (207-238), "parse — aggregates" (241-256),
    "parse — reviver overload" (412-454).
  - `json_malformed.mfb` (67): "parse — security: malformed input traps ErrInvalidFormat"
    (259-314), "deep — heavily nested documents" (657-670).
  - `json_stringify.mfb` (79): "stringify — every variant and the escape set" (317-345),
    "stringify — pretty-printing overloads" (352-407).
  - `json_query.mfb` (150): "get — path reads and ErrNotFound" (457-546),
    "getOr — total fallback" (549-606), "order — …" (614-632),
    "complex — realistic nested document" (635-648).
- **`math.mfb`** (588, 55) — wrapper `TGROUP "Builtin: math"`.
  - `math.mfb` (196): header + abs (12-50), acos (52-68), asin (70-86), atan (88-104), atan2
    (106-139), cos (210-231), sin (541-562), tan (595-616).
  - `math_rounding.mfb` (198): ceil (141-171), floor (272-302), round (462-492), clamp
    (173-208), max (352-362), min (364-415).
  - `math_exp_log.mfb` (148): exp (233-251), "fixed transcendental" (253-270), log (304-341),
    log10 (343-350), pow (417-445), sqrt (564-593).
  - `math_random.mfb` (64): rand (447-460), "scalar array determinism" (494-539).
- **`money.mfb`** (251, 28) — wrapper `TGROUP "Builtin: money"`.
  - `money.mfb` (158): header + "literals and toString", "dimensional arithmetic",
    "arithmetic errors", "conversions out of Money", "conversions into Money" (50-198).
  - `money_rounding.mfb` (100): "money::round" (203-244), "rounding mode governs arithmetic"
    (249-258), "rounding mode state" (263-270), "math surface" (275-308).
- **`primitives.mfb`** (326, 34) — wrapper `TGROUP "Primitives"`.
  - `primitives.mfb` (196): header + "bindings and defaults", typeName, toString, len, toInt,
    toFloat, toFixed, toByte, toMoney, toScalar (25-251).
  - `primitives_operators.mfb` (135): predicates (256-291), "boolean logic" (296-308),
    comparisons (313-335), "string concatenation" (340-350), "promotion matrix" (355-401).
- **`regex.mfb`** (578, 71) — wrapper `TGROUP "Builtin: regex"`. The "match" group (59-282) is
  218 LOC by itself (23 direct TCASEs). It is split across two files, each re-opening
  `TGROUP "match"` (+2 LOC).
  - `regex.mfb` (224): header 1-52 + `TGROUP "match"` holding the TCASEs at 60-128 (literal
    substring … greedy vs lazy) and 251-281 (flags i m s x … match is unanchored) + "pattern
    validation" (531-603) + "security" (610-648).
  - `regex_classes.mfb` (151): `TGROUP "match"` holding the TCASEs at 129-250 (character
    classes … escape sequences: controls and hex) + "unicode positions" (654-678).
  - `regex_search.mfb` (219): "find" (287-359), "findAll" (364-441), "replace" (446-526).
- **`strings.mfb`** (286, 36) — wrapper `TGROUP "Builtin: strings"`.
  - `strings.mfb` (123): header + byteLen (17-25), contains (38-47), endsWith (49-58),
    startsWith (257-266), "startsWithAny / endsWithAny" (85-100), count (114-124), graphemes
    (60-83), "graphemeAt / graphemesCount" (163-169), toBytes (299-318).
  - `strings_transform.mfb` (169): caseFold (27-36), "stripPrefix / stripSuffix" (102-112),
    "left / right" (126-141), repeat (143-148), "padLeft / padRight" (150-161), trimChars
    (171-178), join (180-191), lower (193-202), upper (204-213), normalizeNfc (215-225), split
    (227-255), trim (268-277), trimStart (279-287), trimEnd (289-297).
- **`vector.mfb`** (749, 71) — wrapper `TGROUP "builtin: vector"`.
  - `vector.mfb` (200): header + constants (124-166), toString (596-617), types (619-634),
    angle (61-77), cross (168-211), distance (213-229), dot (231-247), length (249-265).
  - `vector_componentwise.mfb` (154): abs (16-59), max (312-329), min (331-348), scale
    (534-556), clamp_length (79-122).
  - `vector_interpolate.mfb` (87): lerp (267-287), lerp_unclamped (289-310), slerp (558-594).
  - `vector_projection.mfb` (185): normalize (350-393), perpendicular (395-407), project
    (409-448), reflect (450-477), reject (479-518), rotate_2d (520-532).
  - `vector_errors.mfb` (151): errors (641-682), "edge cases" (687-788).

### 4.4 rt-behavior fixtures

**`tests/rt-behavior/datetime/datetime-arith-errors-rt/src/main.mfb`** (275)
- `main.mfb` (50): IMPORT io/datetime/errorCode, helpers `maxI`, `minI`, `bigDt`, `far`,
  `epochLocal`, `report` (1-32), and `quiet` + `main` (290-320).
- `instant_duration.mfb` (115 = 112 + 3 imports): `add`, `subtract`, `plus`, `minus`,
  `negate`, `between`, `instant2`, `instant3`, `instant4`, `instant5`, `duration2`,
  `duration3`, `duration4`, `duration5`, `toMillis`, `toNanos` (33-160).
- `calendar_format.mfb` (116 = 113 + 3 imports): `addDaysOverflow`, `addDaysLocal`,
  `addMonthsOverflow`, `addMonthsLocal`, `startOfDayOverflow`, `startOfDayLocal`,
  `withZoneOverflow`, `withZoneLocal`, `dayOfYear`, `weekday`, `resolve`, `formatToken`,
  `formatOverflow`, `formatDuration`, `toIso1`, `toIso2` (161-289).
- Each new file copies the whole 3-line IMPORT block (io, datetime, errorCode).
- Goldens:
  - `golden/datetime-arith-errors-rt.ast` changes: it gains two file entries, and lines move.
  - `golden/datetime-arith-errors-rt.ir` changes: 1081 `"line"` fields, and function order.
  - `golden/datetime-arith-errors-rt.run` and `golden/build.log` stay byte-identical.

**`tests/rt-behavior/types/union-variant-expression-wrap/src/main.mfb`** (275)
- `main.mfb` (55): IMPORT io/collections, types `Circle`, `Square`, `Shape`, `Hold`, `Box`,
  `show`, `showAll`, `mk`, `mkf`, and globals `LET g`, `MUT gm` (1-57), plus `main` (294-307).
- `cells_access.mfb` (108 = 106 + 2 imports): `ret_call`, `cells_call`, `ret_field`,
  `cells_field`, `ret_get`, `cells_get`, `ret_with`, `cells_with` (58-171).
- `cells_binding.mfb` (116 = 114 + 2 imports): `ret_fallible`, `cells_fallible`, `ret_var`,
  `cells_var`, `ret_ctor`, `cells_ctor`, `ret_matchvar`, `cells_matchvar` (172-293).
- Each new file copies `IMPORT io` + `IMPORT collections`. The moved `cells_*` functions assign
  `gm` (MUT). It is a PUBLIC top-level binding, visible project-wide.
- Goldens:
  - `.ast` and `.ir` change (the file list and lines).
  - `.run` (empty) and `build.log` (which carries the program output, the subject of bug-990)
    stay byte-identical.

**`tests/rt-behavior/lexical/literal-spans-lines-valid/src/main.mfb`** (523): not split.
- Its `ceiling test 523` row in `tests/guards/file_size_ceilings.txt` is replaced by
  `exempt test tests/rt-behavior/lexical/literal-spans-lines-valid/src/main.mfb # main's two 181/183-LOC single-statement list literals are the test subject (bug-821); a statement cannot be split`.
- The same path and reason are added to the `EXEMPT` table in `tools/loc-census/census.py`.

## Phases

> **NOTE — keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes** — never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file — including the
> person who has to decide whether to stop — is entitled to trust that.

Shared gate steps. `M=target/release/mfb`. F changes no Rust, so one release build serves the
whole letter.

- **F0 (once, before Phase 1's first edit):** `cargo build --release` (est. 6 min). Then
  `for p in packages/xml packages/jwt tests/acceptance; do $M test $p | grep '^Tests:'; done > /tmp/f_before.txt`
  (est. 3 min) and
  `bash scripts/test-accept.sh $M /tmp/p176f-base > /tmp/p176f-base.log 2>&1; tail -3 /tmp/p176f-base.log`.
  The full acceptance baseline (est. 40 min) is run now, before any F edit, so that
  pre-existing reds (`.ai/testing-gates.md` § "Acceptance pre-existing reds baseline") are
  known before Phase 5 compares against it.
- **F1:** for each project the phase touched, `$M test <project> | grep '^Tests:'` → equal to
  its line in `/tmp/f_before.txt`. For each split file,
  `grep -c '^ *TCASE ' <old file and its new siblings>` summed → equal to §2's count.
- **F2:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output.
  Every `TIGHTEN`/`STALE` row names a file of this phase. Then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`, rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit the ceiling file with the phase (est. 2 min).
- **F3:** `$M fmt --check <f>` for each new or edited `.mfb` → exit 0 (est. 10 s). Phase 3
  first fixes the one pre-existing failure among the files F edits (`money.mfb`, §2).

### Phase 1 — Package test files

- [ ] Split `packages/jwt/src/test_verify.mfb` into `test_verify.mfb` + `test_verify_keys.mfb`
      per §4.2 (drop `PRIVATE` on the 5 helpers).
- [ ] Split `packages/xml/src/test_write.mfb` into `test_write.mfb` + `test_write_pretty.mfb`
      per §4.2 (drop `PRIVATE` on the 4 helpers).
- [ ] Split `packages/xml/src/test_xpath_edges.mfb` into `test_xpath_edges.mfb` +
      `test_xpath_edges_refusals.mfb` per §4.2 (verbatim PRIVATE `catalog` copy; the commit
      message names it).
- [ ] F2: tighten the three rows.

Acceptance: both packages run the same test count and pass.
  Check: F1 for `packages/xml` (`Tests: 253  Pass: 253  Fail: 0` at planning) and
  `packages/jwt` (`Tests: 102  Pass: 102  Fail: 0`). TCASE sums: 36, 43, 44. Est. 1 min.
  Then `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (est. 3 min). F2, F3.
Commit: —

### Phase 2 — Acceptance suites, part 1

- [ ] Split `tests/acceptance/src/arithmetic.mfb` per §4.3. `arithmetic.mfb:203` still reads
      `expectFloat(fa MOD fn` (`sed -n 203p tests/acceptance/src/arithmetic.mfb`).
- [ ] Split `astrings.mfb` per §4.3.
- [ ] Split `bits.mfb` per §4.3.
- [ ] Split `collections.mfb` per §4.3.
- [ ] Split `crypto.mfb` per §4.3.
- [ ] Split `csv.mfb` per §4.3.
- [ ] Split `encoding.mfb` per §4.3.
- [ ] Split `general.mfb` per §4.3.
- [ ] F2: tighten the eight rows.

Acceptance: the acceptance project runs the same TCASEs and passes.
  Check: F1 for `tests/acceptance` → equal to F0 (`Tests: 785` at planning). TCASE sums: 40,
  71, 55, 35, 91, 32, 41, 22 (est. 3 min). F2, F3.
Commit: —

### Phase 3 — Acceptance suites, part 2

- [ ] Split `tests/acceptance/src/json.mfb` per §4.3.
- [ ] Split `math.mfb` per §4.3.
- [ ] `target/release/mfb fmt tests/acceptance/src/money.mfb` before splitting it. This is a
      pre-existing defect: 10 lines spell the keyword `true` in lowercase, at lines 109-115,
      266, 268 and 306 as of `a3c95fc07`. The fix is behavior-neutral, because keyword
      matching is case-insensitive (`src/docs/spec/language/02_lexical-structure.md:3`). After
      it, `mfb fmt --check tests/acceptance/src/money.mfb` → exit 0.
- [ ] Split `money.mfb` per §4.3.
- [ ] Split `primitives.mfb` per §4.3.
- [ ] Split `regex.mfb` per §4.3 (`TGROUP "match"` re-opened in `regex_classes.mfb`).
- [ ] Split `strings.mfb` per §4.3.
- [ ] Split `vector.mfb` per §4.3.
- [ ] F2: tighten the seven rows.

Acceptance: as Phase 2.
  Check: F1 for `tests/acceptance` → equal to F0. TCASE sums: 54, 55, 28, 34, 71, 36, 71
  (est. 3 min). F2, F3.
Commit: —

### Phase 4 — money-demo

- [ ] Split `examples/money-demo/src/main.mfb` into `main.mfb`, `format.mfb`, `ledger.mfb`,
      `report.mfb`, `screen.mfb` per §4.1, each starting with the 9-line IMPORT block.
- [ ] F2: tighten the `code` row.

Acceptance: money-demo builds for every example target and runs the same on the host.
  Check: `for t in macos-aarch64 linux-aarch64 linux-x86_64 linux-riscv64 windows-x86_64; do $M build -q --target $t examples/money-demo >/dev/null || echo FAIL $t; done`
  → no output. That target list is `scripts/build-examples.sh` `TARGETS`; est. 1 min. Then
  `$M build -q examples/money-demo && (sleep 3; printf q) | script -q /tmp/md.ts sh -c 'stty rows 40 cols 120; examples/money-demo/build/money-demo.out' >/dev/null; echo $?`
  → `0`, and `grep -a -c 'Welcome. Starting balances' /tmp/md.ts` → ≥ 1 (est. 15 s). F2, F3.
Commit: —

### Phase 5 — rt-behavior fixtures and the exemption (largest blast radius)

The only F files with goldens. The re-baseline of two `.ast`/`.ir` pairs follows AGENTS.md's
test/golden rule.

- [ ] Split `tests/rt-behavior/datetime/datetime-arith-errors-rt/src/main.mfb` per §4.4.
- [ ] Split `tests/rt-behavior/types/union-variant-expression-wrap/src/main.mfb` per §4.4.
- [ ] Replace the `ceiling test 523` row for `literal-spans-lines-valid/src/main.mfb` with the
      §4.4 `exempt` row, and add the same path and reason to `tools/loc-census/census.py`'s
      `EXEMPT` table, in this commit.
- [ ] Run the FULL acceptance suite before touching any golden:
      `bash scripts/test-accept.sh $M /tmp/p176f-full > /tmp/p176f-full.log 2>&1`
      (est. 40 min; AGENTS.md: before re-baselining a golden, run the full suite, never one
      module). Compare against the F0 baseline. The mismatch set is the F0 reds plus exactly
      the four goldens `datetime-arith-errors-rt.{ast,ir}` and
      `union_variant_expression_wrap.{ast,ir}`. The `N test(s) ran` count equals F0's. A
      mismatch on any `.run` or `build.log`, or on any other fixture, is a bug introduced by
      this phase: localize it on that one fixture and fix it, then rerun.
- [ ] Re-baseline only those two fixtures' `.ast`/`.ir`, with the AGENTS.md justification in
      the commit message:
      (1) when/why written: `git log --format='%h %s' -- tests/rt-behavior/datetime/datetime-arith-errors-rt/golden tests/rt-behavior/types/union-variant-expression-wrap/golden`;
      (2) what they protect: the AST and IR that the fixture's source lowers to;
      (3) who depends: only `test-accept.sh`/`artifact-gate.sh` read them
      (`grep -rn 'datetime-arith-errors-rt\|union-variant-expression-wrap' scripts tests/guards`
      → no other reader);
      (4) proof the old lines are wrong for the new input: the goldens record `files[].path`
      and per-item `"line"`, and the input's file layout changed. The location-stripped
      check below shows nothing else moved.
      Command: `bash scripts/sync-goldens.sh $M datetime-arith-errors-rt union-variant-expression-wrap`.
- [ ] F2: tighten (the two fixture rows → removed; the exempt row prints nothing).

Acceptance: the two fixtures print byte-identical output, and their AST/IR changed only in
location attribution.
  Check:
  - `git diff --stat -- tests/rt-behavior/*/datetime-arith-errors-rt/golden tests/rt-behavior/*/union-variant-expression-wrap/golden`
    → only the four `.ast`/`.ir` files.
  - Run these before committing the phase. For each `.ir` golden `G`:
    `diff <(git show HEAD:G | sed -E 's/"line": [0-9]+,? ?//g' | sort) <(sed -E 's/"line": [0-9]+,? ?//g' G | sort)`
    → no output.
  - For each `.ast` golden, the same command piped to `grep '^<'` → no output. No item line is
    lost or changed. The `>` lines are only the new files' `"path"`, `"imports"` and import
    entries, and object/array brackets.
  - `bash scripts/test-accept.sh $M /tmp/p176f-two datetime-arith-errors-rt union-variant-expression-wrap literal-spans-lines-valid`
    → 0 mismatches (est. 1 min).
  - `cargo test --test cli_fmt_multiline_brackets` → passes (est. 2 min). F2, F3.
Commit: —

## Validation Plan

- Tests: no TCASE is added or removed. `mfb test` totals for `tests/acceptance`,
  `packages/xml` and `packages/jwt` equal F0's (F1, per phase), and per-file TCASE sums equal
  §2's.
- Coverage check: `.mfb` sources are not in the Rust coverage denominator. The acceptance
  project and packages are covered by `mfb test` itself, and the counts above prove the same
  cases still run. The rt-behavior fixtures are covered by `test-accept.sh`, whose
  `N test(s) ran` count must not move.
- Runtime proof: Phase 4's pty run of money-demo, and Phase 5's byte-identical `.run`/`build.log`
  for the split fixtures, which execute cross-file union wraps and global writes.
- Doc sync: none. No spec citation points into a moved range (§2). The one pinned line,
  `arithmetic.mfb:203`, stays exact, and the xml oracle README stays correct.
- Final gate (once, after Phase 5): the Phase 5 full `test-accept.sh` run is F's whole-suite
  run (F is the one test letter that runs it, because it changes fixture sources with goldens).
  Then `cargo test --test file_size_budget 2>&1 | grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → 0,
  and `grep -E 'tests/acceptance/|packages/(xml|jwt)/|examples/money-demo/|tests/rt-behavior/' tests/guards/file_size_ceilings.txt`
  → exactly the `literal-spans-lines-valid` exempt row (est. 2 min). The Rust workspace suite
  runs once, in plan-176-V.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment non-blank LOC. Scope: `.rs`/`.mfb` under `src/`
  `packages/` `examples/` `repository/` `tests/` — owner, 2026-10-14.
- Inline test code (`TESTING … END TESTING`) counts against the 250 test budget; over 250 it
  moves to its own file — owner, 2026-10-14.
- Generated files are exempt. The guard warns with a banner and never fails. A new module (here
  a new `.mfb` file in the project) is always an acceptable split — owner, 2026-10-14.
- A single test-side unit over 250 gets its own file and an `exempt` row (with reason) in both
  `tests/guards/file_size_ceilings.txt` and `tools/loc-census/census.py`'s `EXEMPT` table —
  owner, 2026-10-14. In F that unit is `tests/rt-behavior/lexical/literal-spans-lines-valid/src/main.mfb`.
  The exempt row is added in F, and the file is not split (author decision, 2026-10-14).
- `.run` and `build.log` goldens stay byte-identical. The `.ast`/`.ir` goldens of the two split
  rt-behavior fixtures are expected to change. They are regenerated for those two fixtures only,
  after the full acceptance suite, with each diff shown to be location attribution only —
  author, 2026-10-14.
- Every new file copies its original's whole IMPORT block. There is no unused-import rule, and
  per-file import pruning is churn — author.
- PRIVATE package test helpers used by both halves become PUBLIC (no collision, measured). The
  colliding `xml` `catalog` is duplicated PRIVATE instead — author.
- Archived bug and plan documents are not edited — author.

## Corrections

## Summary

F is motion of whole MFBASIC declarations and test groups into sibling files of projects that
already glob `**/*.mfb`: 21 files split, 1 exempted, 0 TCASEs changed. Byte-identity cannot
gate it, because file layout reaches `.ncode` through source-location constants (probe). The
gates are behavior: `mfb test` totals, a money-demo build on five targets plus a pty run, and
byte-identical program output for the two split fixtures. The one sanctioned golden
re-baseline (two `.ast`/`.ir` pairs) happens after the full suite and is proven by a
location-stripped diff.
