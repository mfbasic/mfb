# plan-125-B: man iteration 1 — every package and guide topic as a whole, plus the cross-surface consistency review

Last updated: 2026-09-04
Effort: large (3h–1d)
Depends on: plan-125-A (the standards, the harness, the prompts, and the
pilot's measured per-unit cost all exist; without them this letter has no
instrument and no calibration).

Iteration 1 of three. The review unit is **a whole package or a whole guide
topic**, read as one document. This is the only iteration that can see
*coverage* (something a developer needs that no page mentions), *internal
consistency* (siblings explaining one concept two ways), *the overview's
promises against what its functions deliver*, and *ordering and
discoverability*. It is deliberately not a per-claim pass — that is
iteration 2 (letters C–F).

It ends with the **cross-package consistency review**: the one review in the
whole plan that looks at all 41 units at once, before iteration 2 fragments
the surface into 590 independent edits.

Audience, restated because it is the whole point: **the MFBASIC developer,
using and learning the language.** Not a compiler contributor. A sentence
that requires a compiler mental model to parse is a finding here even if it
is true.

References:

- plan-125-A §3.1 (the audience table), §3.2 (why the three lenses differ),
  §3.3 (the per-unit workflow), §4.3 (the fan-out harness), §5 (the
  iteration-1 prompt, run verbatim).
- `.ai/man-content.md` — the standard, as extended by plan-125-A Phase 2 to
  cover the narrative topics.
- plan-108-A §3 (2a) — the memory-vocabulary ban, its rewrite table, and the
  two carve-outs. Unchanged and still binding.
- `planning/plan-125-belongs-in-spec.md` — this letter appends to it.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-A complete | `grep -c '^- \[ \]' planning/plan-125-A-standards-tooling-pilot.md` → `0` | — |
| the pilot's cost table is filled with measured numbers | read plan-125-A Phase 5's table | — |
| `--reconcile` is self-tested | plan-125-A Phase 3 acceptance | — |

## 1. Goal

- **All 39 remaining iteration-1 units** — 30 packages (31 minus `color`,
  done in A's pilot) and 9 guide topics (10 minus `variable`, done in A's
  pilot) — have been through my pass, one `codex exec` review, and apply.
- **Every unit's ledger is in this file**: finding / verdict / evidence, and
  for every rejection **the command that disproves it**.
- **Coverage findings are acted on, not deferred.** Iteration 1's
  characteristic finding is "this package never tells the developer X". A
  missing page or a missing paragraph is a task in this letter.
- **The cross-package consistency review is complete** (§3.2) and its
  findings applied: one concept, one vocabulary, across the whole surface.
- **`planning/plan-125-belongs-in-spec.md` carries every sentence this letter
  cut for being too internal**, with the spec package it belongs to.
- Sweeps still clean for every touched package:
  `./scripts/man-census.sh --memory-scope <pkg>` → 0 unclassified;
  `--scope <pkg>` → 0; `--fill` still 100%.
- `./scripts/doc-review-fanout.sh --reconcile` exits 0 over this letter's
  39-unit list plus the consistency runs.

### Non-goals (explicit constraints)

- Per plan-125-A: no compiler test gates; prose fields and markdown only;
  `git diff` per commit shows string-literal/markdown changes only; no
  renderer or schema changes; the reviewer never commits.
- **Not a per-claim verification pass.** Resist the pull to verify every
  sentence here — that is iteration 2, and doing it now costs the plan a
  whole pass for nothing. Verify what the *package-level* lens surfaces.
- No wording churn on prose that passes the lens.

## 2. Current State

plan-125-A Phase 1 re-censused the surface. Entering this letter, the man
surface is 100% filled, 0 unclassified memory-vocabulary hits, 0 internals
hits — and **never reviewed as whole packages by anyone but plan-108**, which
did not see `color` or `canvas` and did not cover the guide topics at all.

### Measured populations

| What | Count | Command |
|---|---|---|
| iteration-1 units in this letter | **39** | 31 packages + 10 topics = 41, minus `color` and `variable` (plan-125-A Phase 5 pilot) |
| function pages behind those units | 510 | 538 minus `color` 28 |
| guide pages behind those units | 31 | 32 minus `variable` 1 |
| packages plan-108 never reviewed | 2 (`color`, `canvas`) | `color` did not exist; `canvas` is recorded as missed in plan-108-E's Corrections. `color` is done in A's pilot, so **`canvas` is this letter's highest-yield unit** |
| guide topics never independently reviewed | 10 | plan-108 authored `variable` and explicitly excluded the rest (plan-108-A Non-goals) |
| largest units | collections 49 pages, datetime 44, fs 41, strings 39, encoding 30 | `./scripts/man-census.sh --fill` |
| guide topics with subtopics | 4 (types 10 pages, flow 8, tour 6, tooling 2) | `find src/docs/man -name '*.md'` |

### Verified properties

- **The guide topics have never been through any review process** — VERIFIED
  by reading plan-108-A's Non-goals: "`src/docs/man/**` prose guides (tour,
  errors, link, lambda, …) are OUT of plan-108's scope", with `variable` the
  single carve-out. They are the pages a learner reads first and the least
  audited material in the product.
- **`canvas` carries 106 type descriptions** (`--fill` TYPES column
  `106/106`) — by far the largest `types` page, and one no reviewer has seen.
  Its unit is closer in size to a small package than to a types page.
- UNVERIFIED: whether any package regressed since plan-108. Measured by this
  letter, assumed neither way.

## 3. Design Overview

### 3.1 Order of units

Highest-uncertainty first, so a systemic finding is discovered while there are
still 38 units to apply it to:

1. **`canvas`** — never reviewed, largest types page.
2. **The 9 guide topics** — never reviewed, and they set the vocabulary every
   package page borrows. A terminology decision made here propagates.
3. **The packages that changed most since plan-108** — `datetime`, `http`,
   `process`, `json`, `tls`, `term`, `astrings`, `crypto` (`git log
   --since=2026-08-31 --name-only --format='' -- src/codegen/builtins | grep
   -oE 'builtins/[a-zA-Z]+/' | sort | uniq -c | sort -rn`).
4. The remainder, largest first.

### 3.2 The cross-package consistency review

Cannot be one run over 51,540 lines. It is run over a **condensed artifact** —
every package overview plus every `types` page plus every guide topic
overview (a small fraction of the surface, and the part where vocabulary is
actually *established*) — in four dimension-scoped runs:

1. **Concept vocabulary** — is one thing called one thing? (handle vs
   resource; fails vs errors vs raises; index vs position; byte vs character
   vs grapheme; empty vs blank).
2. **Overview shape** — do the 31 overviews answer the same questions in the
   same order, so a developer learns to read them?
3. **Guide↔package agreement** — do the 10 topics and the package pages agree,
   especially `types`, `errors`, `flow` and `variable` against the packages
   that lean on them?
4. **Handle/resource contract** — the packages that own `RES` handles (`fs`,
   `io`, `tcp`, `udp`, `tls`, `net`, `process`, `audio`, `canvas`, `term`)
   must state the open/close contract identically. plan-108-F recorded this
   as the sharpest divergence test and it has 27 packages of churn since.

Each dimension's findings are applied across **all** affected units, not just
the one that surfaced them — plan-108-B's recorded lesson: *a reviewer finding
is usually a class, not an instance.*

### 3.3 Risk

- **Scope creep into iteration 2.** The biggest cost risk in this letter is
  verifying claims that iteration 2 will verify anyway. Held by the prompt
  (plan-125-A §5 iteration-1 prompt states the lens and forbids per-claim
  verification) and by the ledger recording finding *class* per unit.
- **A coverage finding that is really a missing feature.** "The package never
  says how to X" sometimes means the package cannot X. That is a product
  observation, recorded in the ledger and *not* documented as if it worked.
- Network-package units: the reviewer cannot bind sockets (plan-108-C's
  lesson); any probe for `tcp`/`udp`/`tls`/`net`/`http` is run by the main
  thread.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. `- [~]` for partial, with one line on what remains.
> `- [x] ~~text~~ — moot: <evidence>` rather than deleting. Fill `Commit:`
> the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — canvas and the nine guide topics (11 units)

The never-reviewed material, and the material that sets vocabulary for
everything after it.

- [x] `canvas` (19 function pages + overview + a 106-description types page).
      My pass: `--memory-scope canvas` 0 unclassified, `--scope canvas` 0,
      22/22 examples compile (compile-only: app-mode programs take over the
      desktop). Codex review: 1 finding, confirmed and applied (ledger below).
- [x] `tour`, `types`, `flow`, `errors`, `lambda`, `link`, `optimizations`,
      `tooling`, `unicode` — each as a whole topic including its subtopic
      pages. My pass (C-2, C-3, C-4): whole-surface `--memory-scope`
      109 unclassified → 0, `--scope` 9 → 0; 38 citation markers removed; one
      broken example fixed (`flow pipeline`) and one renderer-dropped syntax
      block fixed (`types string`). Codex reviews: 23 findings, 22 confirmed and
      applied, 1 rejected (ledger below).
- [x] Record every terminology decision made here in a running table in this
      file; later phases conform to it rather than re-deciding. Table below;
      the reviews added none (the applied prose uses the table's words).
- [x] Append every "belongs in spec" cut to
      `planning/plan-125-belongs-in-spec.md`. Rows 4–19 from my pass
      (`types` storage layout ×10, `types` monomorphization, `lambda` ×2,
      `optimizations` stage legend, `link` resolver and loader); row 20 from
      the reviews (`canvas::destroyFont`'s scene-id sentence).

#### Terminology table (conform to this; do not re-decide)

| Concept | Write | Not | Decided from |
|---|---|---|---|
| a type the language provides | **built-in** | "compiler-owned" | `types`, `errors`, `lambda`, `pair`, `partition` |
| a value ceasing to exist at scope end | **goes away when its scope ends** | "dropped", "reclaimed", "lexical drop" | `tour`, `errors`, `types` |
| a `RES` handle closing at scope end | **closed when its scope ends** | "closed by lexical drop", "owned by this scope" | `tour` ×6, `errors` |
| a second name for one open handle | **alias** (for `RES` only) | "owner", "the same owner" | `.ai/man-content.md` §4.1 |
| what `thread::send` does to a value | **hands over** (the receiver has its own copy) | "moves", "owns its copy" | `tour` ×5 |
| what a collection value contains | **holds** its items | "owns its contents", "owned block" | `types list`/`map`/`set` |
| a `MUT` collection updated efficiently | **the change can be made in place** | "uniquely-owned … live buffer" | `types list`/`map`/`set` |
| a close function's parameter | **takes … and closes it** | "consumes" | `link` |
| a closure's captured `LET` | **gets its own copy** | "by value", "deep-copies" | `lambda` |
| a `forEach` lambda changing an outer `MUT` | **changes the outer binding itself** | "borrow", "loaned" | `lambda` |
| the unit a `String` is measured and split in | **Unicode scalar** | "character" (ambiguous with a grapheme) | Phase 4 vocabulary #1: `tcp`, `tls`, `udp` |
| a second explicit close of a handle | **raises `ErrResourceClosed`** (state the exception where one exists, e.g. canvas `destroy*`) | "the same contract every resource has" unless it is | Phase 4 handles #1 |
| a handle that cannot cross threads | **stays on the thread that opened it** | "thread-local", "not sendable" | `types` pages of `audio`, `canvas`, `process` |
| a handle that can | **may be handed to another thread with `thread::transfer`** | "sendable", "moved to a thread" | `types` pages of `fs`, `tcp`, `udp`, `tls` |
| a resource overview's lifecycle paragraph | in this order: **scope close; early close; second close; later use; thread rule** | per-package ad-hoc order | Phase 4 handles #2 |
| map iteration order | **implementation-defined but stable for an unchanged map** | "insertion order" as a promise | Phase 4 guide-package #2 |
| a package overview's import line | **"Import it with `IMPORT <pkg>`"** | leaving it implicit | Phase 4 overview-shape #1 |

#### Phase 1 ledger — Codex iteration 1 (`planning/plan-125-findings/B-phase1/`)

Manifest: 10/10 units `exit 0`, all `clean`, 60–217 s each (C-1: 10 units, not
11). 24 findings: 23 confirmed and applied, 1 rejected.

| Unit | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| canvas | 1 | CONFIRMED | `func_measure_text.rs:lower_font_bytes` raises `ErrResourceClosed` before measuring; `gen_font_table.rs` "text still naming a released font draws empty" | `measureText` and `destroyFont` DESC: a closed font draws as nothing but raises on measure; scene-id sentence cut (belongs-in-spec row 20) |
| errors | 1 | CONFIRMED | spec 08 §8.3/§8.4; probe `/tmp/p125-ex/baretrap` printed `8080` for a bare `TRAP` | bare-`TRAP` paragraph + example after Trap outcomes |
| errors | 2 | CONFIRMED | spec 08 §8.4 and §8.6 rule 11; same probe printed `caught 77050002` for `divide(total, 1 / count) TRAP(e)` | inline-TRAP operator coverage and `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL` |
| errors | 3 | REJECTED | `mfb man general error` exit 0 and `mfb man flow match` exit 0 at HEAD (`/tmp/p125-ex/facts.sh`); the reviewer's worktree binary predated guide-topic sub-pages | — |
| flow | 1 | CONFIRMED | `/tmp/p125-pr1` printed `set loop: 3 1 2`; spec 12 "yields each element `T` once, in insertion order" | `flow` Topics, `forEach` (Set loop), `tour` (lists, sets, and maps; `Set OF T` in the collections list) |
| flow | 2 | CONFIRMED | `/tmp/p125-pr1` printed `pipeline twice: 1:2 calls=2` | `pipeline`: each `_` gets its own copy; evaluated per occurrence |
| flow | 3 | CONFIRMED | `/tmp/p125-pr1` printed `zero step passes before exit: 3` | `for`: a non-constant zero `STEP` never advances |
| lambda | 1 | CONFIRMED | `mfb man thread start`; reviewer probe ran a non-`EXPORT` `ISOLATED FUNC` | `lambda` entry-point sentence; the same wrong "exported" claim fixed in `tour`, `tour c`, `tour java`, `tour typescript` |
| lambda | 2 | CONFIRMED | `/tmp/p125-ex/adder` built and printed `12` | `makeAdder` example with output |
| lambda | 3 | CONFIRMED | `.ai/man-content.md` §4.5 | `mfb man variable` inline and in See also (+ `thread start`) |
| link | 1 | CONFIRMED | spec 17 CSTRUCT / BIND IN / BIND STATE / CBuffer sections | new *Structs and buffers* section, forms list, `NATIVE_BUFFER_INVALID` and `ErrNativeBufferOverrun` rows |
| link | 2 | CONFIRMED | `src/ast/link_items.rs:parse_link_function` (plan-50-H); `tests/rt-behavior/native/native-link-free-rt/src/main.mfb` uses `db OUT CPtr` + `RETURN db` | `RETURN <expression>` paragraph; the page's own sqlite example used `return OUT CPtr`, which no longer parses — now `db OUT CPtr` + `RETURN db`; four Diagnostics rows that said "result marker"/`return` |
| optimizations | 1 | CONFIRMED | `src/optimizer/catalog.rs:rows` interleaves NIR and MIR rows | legend: rows are not in running order |
| optimizations | 2 | CONFIRMED | `mfb test --help` lists `-O` and `-v` | Synopsis, `-v` paragraph, See also |
| tooling | 1 | CONFIRMED | `mfb --help` lists `audit`; `mfb man tooling audit` was unknown | new `tooling/audit.md` (auto-discovered); overview names both commands. Writing it found bug-604 (`mfb audit --help` misdescribes `--locked`) — filed, not fixed |
| tooling | 2 | CONFIRMED | `src/cli/fmt.rs:parse_indent` accepts 0..=256 | `fmt` `--indent` row |
| tour | 1 | CONFIRMED, suggestion corrected | there is no `mfb run` (`mfb --help`); `mfb init` writes a hello-world `src/main.mfb` | route to `mfb init` / `mfb build` / `mfb test`, `mfb man tooling` |
| tour | 2 | CONFIRMED | `01_c`/`02_java`/`03_go` link `mfb man variable` | added to `04_typescript`, `05_python` |
| types | 1 | CONFIRMED | `mfb man flow forEach` renders the MapEntry contract | MapEntry bullet + See also |
| types | 2 | CONFIRMED | `/tmp/p125-ex/deftypes` built and printed `square 3`, `green` | *Defining types* section (TYPE/UNION/ENUM program, WITH → `variable`, RESOURCE → `link`) |
| unicode | 1 | CONFIRMED | `strings::displayWidth` DESC; reviewer probe `displayWidth("日本語")` = 6 | terminal-columns paragraph |
| unicode | 2 | CONFIRMED | all seven targets exit 0 (`/tmp/p125-ex/verify.sh`) | See also: `types string` + six `strings` pages |

Handed to the spec letters (spec defects seen while verifying; not man work):
spec 17's opening `sqlite3_open` example has no `RETURN db`, which its own rule
(`NATIVE_ABI_NO_RESULT`) requires; its `readFrames` example binds parameter
`file` to ABI slot `sndfile`; and its `CBuffer` implementation-status note says
`BUFFER`/`LENGTH` do not ride the `.mfp`, but plan-58-C is in
`planning/completed/` and spec package 08 documents BR version 6 carrying them.

After apply (release build of this commit): whole-surface `--memory-scope` 0
unclassified, `--scope` 0; leak check 0 rendered, 0 source spans; topic programs
lambda 2/2, types 2/2, flow 10/10, tour 1/1 built and ran; canvas 22/22 compile;
13/13 new cross-references exit 0.

Acceptance: 11 units in the manifest with `exit 0`, no `FAILED`, no `DIRTY`;
each has a ledger in this file with a verdict per finding and a disproving
command per rejection; `--memory-scope`/`--scope` clean for `canvas`;
`mfb man <topic>` renders for all nine.
Commit: 425093122 (my pass), 83e4199d2 (Codex reviews applied)

### Phase 2 — the eight most-changed packages (8 units)

- [x] `datetime`, `http`, `process`, `json`, `tls`, `term`, `astrings`,
      `crypto` — the packages with the most commits since plan-108 closed.
      My pass done for all eight (sweeps 0, examples as measured, commit
      history read). Codex: all eight reviewed. `term`, `astrings` and `crypto`
      first failed on the usage limit and were re-dispatched after the reset
      (C-5).
- [x] For each: reconcile the page against what actually changed
      (`git log --since=2026-08-31 -- src/codegen/builtins/<pkg>`) — a prose
      field that was not touched by a behavior change is the likely defect.
      Done through the eight reviews; ledger below.

#### Phase 2 ledger — Codex iteration 1 (`planning/plan-125-findings/B-phase2/`)

Manifest: 8/8 units' last rows `exit 0`, `clean` (52–111 s). The retry's
`--reconcile` over `B-phase2-retry.txt` reports `unaccounted=0`; its 5
"orphans" are the first batch's five units, which are not in the retry list.
14 findings: 14 confirmed and applied, 0 rejected.

| Unit | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| datetime | 1 | CONFIRMED | the overview contradicts itself ("referenced bare (`datetime::Instant`…)"); reviewer probe: bare `Instant` is `SYMBOL_UNKNOWN_TYPE` | "always written package-qualified" |
| datetime | 2 | CONFIRMED | `func_date.rs`/`func_time.rs` build from fields alone; reviewer probe printed `2026 9` | `DateTime` is the projection; `Date`/`Time` are standalone |
| http | 1 | CONFIRMED | `mfb man http startRead`: "The whole request is written before `startRead` returns", with a 30-second connect deadline | overview: `startRead` connects and sends; the other four don't block |
| http | 2 | CONFIRMED | `func_response_default.rs` DESC | `responseDefault` added to the constructor list |
| json | 1 | CONFIRMED | `mfb man json stringify` documents Integer and String `indent` overloads | "compact by default, indented when given an indent" (twice) |
| json | 2 | CONFIRMED | the same overview later says a step is an array index on a `JsonArr` | "object keys and array indexes" |
| process | 1 | CONFIRMED | `func_did_signal.rs` DESC: a Windows NTSTATUS error severity maps to `Signal.Error` | overview's Windows sentence |
| tls | 1 | CONFIRMED | `tls/func_close.rs` declares `ErrResourceClosed` — "a second close raises"; `tcp/func_close.rs:34` also raises, so the overview's "unlike `tcp::close`" contrast was wrong too | "As with `tcp::close`, calling it again … raises `ErrResourceClosed`" |
| tls | 2 | CONFIRMED | `tls::close` and `tls::poll` already say "`tls::connect` or `tls::accept`"; reviewer probe compiled `tls::read`/`write` on an accepted socket | `read` ×1 and `write` ×2 `sock` descriptions |
| term | 1 | CONFIRMED (retry, C-5) | `func_is_on.rs` DESC: `on`, `isOn` and `didResize` are the three ungated calls, and `terminalSize` raises `ErrUnsupported` while off | `term::off`: inactive-state sentence names `didResize` and routes to `mfb man term isOn` |
| astrings | 1 | CONFIRMED (retry) | `mfb spec stdlib astrings` "Attribute-aware `strings::` overloads" (Tier-A/Tier-B, and `&`); reviewer probe printed `ell!`/`styled` | overview paragraph: reading vs changing `strings::` functions, dropped styling for case/NFC, `&` |
| astrings | 2 | CONFIRMED (retry) | `helper_md_state_at.rs` BODY comment: only `FontSize` renders; `Foreground`/`Background` carry no marker | `toMarkdown` INTRO |
| crypto | 1 | CONFIRMED, wider (retry) | `mfb man crypto uuid4` → `String`, `randomInt` → `Integer`; `/tmp/p125-ex/crypto-facts.py`: `hash`, `hmac` and `seal` have `String` data forms, but `pbkdf2` has **one** `List OF Byte` form, so the overview's "hash/HMAC/PBKDF2 … also accept a `String`" was false too | overview types paragraph |
| crypto | 2 | CONFIRMED (retry) | `func_encrypt.rs`/`func_decrypt.rs` (HPKE), `func_exchange.rs` (X25519/X448), `func_convert.rs` (Ed25519/Ed448 → key agreement) | overview capability list + routing paragraph |

After apply (release build): whole-surface `--memory-scope` 0 unclassified,
`--scope` 0; `mfb man <pkg> --all` exit 0 for all eight. Examples after the
retry's edits: `astrings` 17/17 and `crypto` 31/31 build and run; `term` 43/43
compile (running would take over the terminal).

Acceptance: 8 units reconciled in the manifest; ledgers recorded; sweeps clean
for all eight.
Commit: a1cd1f9f7 (first five), 17d9cac4c (retry three)

### Phase 3 — the remaining 20 packages

- [x] `collections`, `fs`, `strings`, `encoding`, `math`, `vector`, `os`,
      `general`, `bits`, `io`, `audio`, `tcp`, `testing`, `thread`, `udp`,
      `net`, `csv`, `money`, `regex`, `app`, `errorCode` (21 names; `color` is
      A's pilot — 20 units remain here after Phase 2's eight).
      **My pass done:**
      - Examples: every package's build (`/tmp/p125-B-phase3-examples.log`). 19 also
        run clean; `app` and `audio` compile only (desktop/audio); `io` 22/22
        with `STDIN_FILE`.
      - Word classes the census does not carry: `collections` "payload"/NaN/
        non-`Ok` on 10 pages, `fs` "packed data" on 8, `distinct` internals and
        a run-together paragraph.
      - Rendered dotted type names: 6 cells, bug-605.
      - Belongs-in-spec rows 21–26.

      Codex: all 21 reviewed, 37 findings, ledger below.
- [x] Apply every class-level finding from Phases 1–2 across these units as
      part of the pass, not as a separate sweep. One grep over all registry
      prose covered the Phase 1–2 classes:
      - thread entries described as "exported" (Phase 1 lambda #1);
      - the nonexistent `mfb run` (Phase 1 tour #1);
      - the removed `RESULT` clause (Phase 1 link #2);
      - `FOR EACH` described as List and Map only (Phase 1 flow #1);
      - "referenced bare" (Phase 2 datetime #1);
      - an "unlike `x::close`" contrast (Phase 2 tls #1).

      Two more hits, both fixed:
      - `os::sleep`'s example said a thread entry "must be an exported
        `ISOLATED FUNC`";
      - the `canvas` overview said its value types are "referenced bare" while
        spelling them `canvas::DrawItem`. It also carried the scene-id reason
        (belongs-in-spec row 27).

      Phase 3's own new class, a sentence contrasting a spelling with itself
      (`money` #1), found 3 more in `app`.

#### Phase 3 ledger — Codex iteration 1 (`planning/plan-125-findings/B-phase3/`)

Manifest: 21/21 units `exit 0`, all `clean`; `--reconcile` `unaccounted=0
orphans=0`. 37 findings. 32 were confirmed and applied as prose; 5 were confirmed
as compiler or renderer defects and filed as bugs, with no prose change that
would hide them. None rejected.

| Unit | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| collections | 1 | CONFIRMED | the function table lists `toSet`/`union`/`isSubset`…; the reviewer ran their examples | INTRO "List, Map, and Set helper functions"; overview gains the Set group |
| encoding | 1 | CONFIRMED → **bug-606** | `/tmp/p125-ex/pctdecode`: `percentDecode("%")` and `formUrlDecode("%")` TRAP `77050003`, yet both descriptors declare `errors: vec![]` | none: the declared-error list is compiler data |
| fs | 1 | CONFIRMED | `mfb man fs openWithin` returns `fs::File`, but it is missing from the overview's list | overview handle list + purpose clause |
| fs | 2 | CONFIRMED | `writeTextAtomic`/`writeBytesAtomic` pages qualify: "atomic when the host filesystem supports atomic rename" | overview sentence conditional |
| fs | 3 | CONFIRMED | `gen_open.rs:lower_fs_open_within_helper` compares `openat2`'s errno with `38` (`ENOSYS`), then does a plain open | `openWithin` DESC: Linux fallback caveat |
| os | 1 | CONFIRMED | `emit_env_lock`/`unlock` in `setEnv`, `unsetEnv`, `environ`, `hasEnv`, `userName`, and `getEnv`'s path | overview + `setEnv` + `unsetEnv`: the env calls take turns |
| strings | 1 | CONFIRMED | `displayWidth`, `padLeftToWidth` and `padRightToWidth` are registered but missing from the catalog | catalog gains the terminal-width group |
| vector | 1 | CONFIRMED | `mod.rs` registers `forward` for 3D/4D only (42 = 5 × 9 − 3) | overview constants sentence |
| vector | 2 | CONFIRMED | reviewer probe ran `vector::zeroFloat3`, `upInteger2`, `forwardFixed3` | overview gives the naming rule; the renderer shows no constants → **bug-609** |
| vector | 3 | CONFIRMED | `length` and `distance` pages also say "never raises `ErrInvalidArgument`" | `cross` no longer claims "only" |
| math | 1 | CONFIRMED | `floor`/`ceil`/`round` list forms return `List OF Integer` | overview |
| math | 2 | CONFIRMED | `entry.rs` seeds each thread's PCG64 from OS entropy; reviewer probe drew differently across two runs | `seed` DESC |
| general | 1 | CONFIRMED | `general/mod.rs:expected_arguments` holds the real accepted types; the pages show one form each | overview: accepted source types per conversion |
| general | 2 | CONFIRMED | `mfb man general toInt`: "`isNumeric` is not a safe guard for `toInt`"; `isNumeric`'s example guarded `toInt` | DESC: a guard for decimal conversions only; example uses `toFloat` (output unchanged) |
| general | 3 | CONFIRMED | `expected_arguments`: `IS_POSITIVE \| IS_NEGATIVE \| IS_ZERO => "Integer, Float, or Fixed"` | `isPositive`, `isNegative` |
| bits | 1 | CONFIRMED | `band`/`bor`/`bxor`/`bnot` take `Integer` | "bitwise operations" |
| bits | 2 | CONFIRMED | reviewer probe: `40 AND -40` is `TYPE_BINARY_OPERATOR_MISMATCH` | `ctz`: `bits::band(value, -value)` |
| bits | 3 | CONFIRMED | `sra`'s declaration carries no sign annotation | overview wording |
| io | 1 | CONFIRMED | `mfb spec diagnostics error-codes` has `ErrEndOfFile`/`ErrWriteFailed`; `ErrEof`/`ErrOutput` do not exist | 9 names across 8 pages |
| audio | 1 | CONFIRMED | `func_xruns.rs` DESC: "never raises `ErrAudioUnavailable` even on a Linux host without ALSA" | overview: device-needing calls; `render` and `xruns` excepted |
| audio | 2 | CONFIRMED | `func_play.rs` mixes MML tracks (`__audio_mmlMix`) | overview: no *raw-PCM* mixing |
| testing | 1 | CONFIRMED → **bug-607** | `mfb man testing expectEqual` and `mfb man general len` render `testing::`/`general::` declarations | none: renderer |
| testing | 2 | CONFIRMED | spec 22 Structure/Running/Coverage; `/tmp/p125-ex/testfw` ran the new example: `Tests: 2 Pass: 2 Fail: 0` | overview section. My first draft used `expectTrap(10 / 0)`, which is `TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE`; the probe caught it and it is now `toInt("x")` |
| thread | 1 | CONFIRMED | `lowering.rs` routes worker `receive`/`accept` through the cancellation-checking helper; `tests/rt-behavior/threads/thread-queue-timeout-cancel` exists; reviewer ran it: `receive interrupted` | overview, `isCancelled`, `cancel` |
| tcp | 1 | CONFIRMED | `func_poll.rs`: the list form returns a socket or raises `ErrTimeout` | `setReadTimeout` DESC |
| tcp | 2 | CONFIRMED | reviewer probe: passing an address compiles without `IMPORT net`; reading `.port` does not | overview |
| udp | 1 | CONFIRMED → **bug-608** | 9 forms declare `errors: vec![]`; `gen_io.rs` raises 7 distinct errors | none: compiler data |
| net | 1 | CONFIRMED | reviewer probe: `net::toString(u)` "does not export"; the universal `toString` renders a URL | `Url` record description |
| net | 2 | CONFIRMED | `grep -rln 'net::Address\|ADDRESS_TYPE' src/codegen/builtins/http` is empty | overview names `tcp`, `udp`, `tls` |
| csv | 1 | CONFIRMED | reviewer probe: `parse("a,b", ",;")` gives 2 fields; `stringify` writes `a,;b` | `parse`, `parseStream` |
| money | 1 | CONFIRMED | the sentence read "write `money::Rounding.Banker`, not `money::Rounding.Banker`"; reviewer probe: bare `Rounding` is `SYMBOL_UNKNOWN_TYPE` | `getRounding`, `setRounding` |
| regex | 1 | CONFIRMED | `mfb man regex language` is an unknown function; `mfb spec stdlib regex` exits 0 | route fixed |
| regex | 2 | CONFIRMED | `mfb man regex count` has `[start AS Integer]` | `count` added to the list |
| app | 1 | CONFIRMED | reviewer probe built `app::setMode(app::Mode.Canvas)` | `setMode` DESC + `mode` parameter |
| app | 2 | CONFIRMED | `mfb man canvas` is set up by `setMode(app::Mode.Canvas)` | `setMode` routes to `mfb man canvas` |
| errorCode | 1 | CONFIRMED → **bug-609** | 52 `add_constant` rows, none rendered | none: renderer (the overview already routes to `mfb spec diagnostics error-codes`) |
| errorCode | 2 | CONFIRMED → **bug-609** | `src/cli/man.rs`'s types fallback always says "list its functions" | none: renderer |

Class sweep from `money` #1: a sentence contrasting a spelling with itself.
`python3 /tmp/p125-ex/selfcontra.py` over all registry and topic prose found 3
more, all `app` (`setMode`, `getMode`, overview). All fixed.

Acceptance: every remaining unit in the manifest with `exit 0`; ledgers
recorded; `./scripts/man-census.sh --fill` still 100%, `--memory-scope` 0
unclassified and `--scope` 0 across the whole surface.
Commit: 17d9cac4c (my pass, part 2), 7f4ff371b (reviews applied)

Acceptance measured at `7f4ff371b`:
- 21/21 units `exit 0` and clean, `--reconcile` `unaccounted=0`.
- `./scripts/man-census.sh --fill`: `TOTAL 544 544 544 544 1278/1278`, and "pages with
  neither Description nor Examples: 0".
- `--memory-scope` 0 unclassified, `--scope` 0.

### Phase 4 — the cross-package consistency review

- [x] Build the condensed artifact (§3.2): all 31 overviews + all 20 types
      pages + all 10 topic overviews, concatenated deterministically.
      `./scripts/man-manual.sh --condensed` (C-6, `29da2a4ef`): 62 pages
      (31 + 21 types + 10), 7,205 lines, byte-identical across two runs, empty
      stderr. The reviewers regenerate it themselves in `{{SCRATCH}}`.
- [x] Run the four dimension-scoped reviews. `B-phase4` manifest: 4/4
      `exit 0`, `clean` (112–212 s), prompt `man-consistency.txt` (C-6).
- [x] Apply each finding **as a class** across every affected unit; record in
      the ledger which units each class touched. Ledger below.

#### Phase 4 ledger — cross-package consistency (`planning/plan-125-findings/B-phase4/`)

7 findings in 5 classes. All were confirmed and applied; one also filed as a bug.

| Dimension | # | Verdict | Evidence | Class applied to (units) |
|---|---|---|---|---|
| guide-package | 1 | CONFIRMED → **bug-610** | spec §15: "a second close is a defined no-op reported as `ErrResourceClosed`"; `canvas/func_destroy_image.rs`/`func_destroy_font.rs` do an unconditional closed-flag store and say "the same contract every resource has", but `fs`/`tcp`/`udp`/`tls`/`audio` close raise | `canvas destroyImage`, `canvas destroyFont`: second close documented as the exception it is; "same contract every resource has" removed |
| handles | 1 | CONFIRMED (same class) | as above | same two pages + `canvas` overview |
| guide-package | 2 | CONFIRMED | `mfb spec language collections` §12: map order "implementation-defined stable"; `types map` promised "the same insertion order" | `types map`: "the same order". `collections keys`/`values` already qualify insertion order as "the current implementation's behavior rather than a guarantee": checked, unchanged |
| handles | 2 | CONFIRMED | only `tls`'s overview answered all five lifecycle questions. Thread rules from each `types` page: `fs`/`tcp`/`udp`/`tls` "May be handed to another thread with `thread::transfer`"; `audio` streams, `canvas::Image` and `process::Process` "Stays on the thread that opened it". `mfb man process detach`: "every later `process::` call on it … raises `ErrResourceClosed`" | overviews of `audio`, `canvas`, `fs`, `tcp`, `udp`, `process` get scope close, early close, second close, later use and thread rule (`tls` already complete). `canvas::Font` has no stated thread rule, so none is invented |
| handles | 3 | CONFIRMED | `fs` overview "Using a `File` after it is closed fails" vs `mfb man variable` "refused at compile time … reported as `ErrResourceClosed`" | `fs` overview names both outcomes |
| overview-shape | 1 | CONFIRMED | the reviewer's `rg` split: 23 overviews name their own `IMPORT`; `astrings`, `process`, `tls` none; `canvas` only `IMPORT color` | `astrings`, `canvas`, `process`, `tls`: "Import it with `IMPORT <pkg>`" |
| vocabulary | 1 | CONFIRMED | `grep -ci 'Unicode scalar'` 23 vs "character boundary" 2 and "character in half" 1 in the condensed artifact; `strings` establishes Unicode scalars | `tcp` overview + `tcp read`, `tls` overview + `tls read`, `udp` overview (5 prose sites; 2 code comments left alone) |

Found while applying, not raised by a reviewer: the `canvas` overview's `Paint`
fragment wrote `LET glow AS Paint = … { blend := BlendMode.Add }`. A
compile-only probe (`/tmp/p125-ex/canvaspaint`, `mfb build --app`) failed with
`SYMBOL_UNKNOWN_TYPE` for `Paint` and `SYMBOL_UNKNOWN_IDENTIFIER` for
`BlendMode`. It now reads `canvas::Paint` / `canvas::BlendMode.Add`, and the probe
builds. `grep -E '(AS|OF) (Paint|DrawItem|…)\b'` over canvas finds bare names only
in the package's internal MFBASIC bodies, where they are correct.
- [x] Re-run `--reconcile` over the full 39-unit list plus the four
      consistency runs. The harness keeps one manifest per letter, so the 39
      units (`planning/plan-125-units/B-all.txt`) reconcile as three runs:
      `B-phase1` 10, `B-phase2` 8 and `B-phase3` 21, each `unaccounted=0
      orphans=0`. The consistency runs: `B-phase4` 4, `unaccounted=0
      orphans=0`.

Acceptance measured: four consistency runs in the manifest; all 7 findings
confirmed, each with the units it was applied to (ledger above); every
`--reconcile` exits with `unaccounted=0`. The terminology table gained seven
rows from this phase, and letters C–G conform to it. After apply:
`--memory-scope` 0 unclassified, `--scope` 0, leak check 0/0, and all 10 touched
units render.

Acceptance: four consistency runs in the manifest; every finding has a verdict
and, if confirmed, a list of the units it was applied to; `--reconcile` exits
0; the terminology table in this file is complete and is what letters C–G
conform to.
Commit: 29da2a4ef (C-6 prompt + artifact), 1bcd9d803 (findings applied)

## Validation Plan

- Tests: none (man prose). If a fix changes text pinned by
  `tests/cli/cli_man_summary_plain.rs` or `tests/cli/cli_canvas_man_examples_compile.rs`,
  update the pin in the same commit and run that test alone.
- Coverage check: `./scripts/doc-review-fanout.sh --reconcile` over this
  letter's unit list — a clean letter means every unit *ran*.
- Runtime proof: `mfb man <pkg> --all`, `mfb man <pkg> types`,
  `mfb man <topic>` for every touched unit.
- Doc sync: `planning/plan-125-belongs-in-spec.md` appended; the terminology
  table in this file kept current.
- Acceptance: the four sweeps (`--fill`, `--memory-scope`, `--scope`,
  `--reconcile`) at their targets.

## Open Decisions

- **Does a coverage gap get a new function page, or a paragraph on an existing
  one?** — Recommend a paragraph wherever the information belongs to an
  existing member, and a new page only when a registry member genuinely has no
  page. A new page for a member that does not exist is a feature request, not
  a doc fix, and is recorded rather than written.

## Corrections

<!-- Filled in DURING execution. -->

### C-1 — every count in §2 drifted, and Phase 1 is 10 units, not 11

Measured 2026-09-12 on `worktree-P-125` after merging main (`ecb35180d`):

- **function pages behind this letter's units: 516, not 510.**
  `./scripts/man-census.sh --fill` → `TOTAL 544 …`, minus `color`'s 28. The
  plan wrote "538 minus `color` 28".
- **Phase 1 is 10 units, not 11.** `canvas` plus "the nine guide topics" is 10;
  `planning/plan-125-units/B-phase1.txt` lists exactly those 10.
- **Guide pages behind this letter's units: 31** — unchanged
  (`./scripts/man-census.sh --topics` → 32, minus `variable`).
- **The unit total stays 39** (30 packages + 9 topics). Only the page count
  behind them moved.

### C-2 — the example instrument could not see nine of the ten topics' programs

plan-125-A added `--topic` to `scripts/man-run-examples.sh` and proved it on
`variable` (10/10). Run over this letter's nine topics it reported
**`examples: 0` for every one of them**. Measured cause:
`grep -h '^```' src/docs/man/*/*.md | sort | uniq -c` → **258 untagged fences
and 10 ```basic**, and all ten ```basic fences are in `variable`. The extractor
accepted only ```basic, so it silently skipped every program in `tour`,
`types`, `flow`, `errors`, `lambda`, `link`, `optimizations`, `tooling` and
`unicode` — the exact "0 reads as nothing to check" failure `--topic` exists to
prevent.

Fixed before this letter's pass counts, in two steps, both measured:

1. **IMPORT-led untagged fences.** Accepting ```basic **or** an untagged fence
   whose first line is `IMPORT` found 33 blocks — and **18 failed to build**
   (`tour` 12 of 13, `types` 1 of 2, `flow` 1 of 10, `errors` 1 of 1).
2. **…that also define `main`.** Reading every failure: `tour` #1/3/5/7/9 and
   `errors` #1 were top-level fragments (`MFB_PARSE_UNEXPECTED_STATEMENT`, no
   `SUB main`); `tour` #2/4/6/8/10 and `types` #2 were companion **package**
   files (`EXPORT ISOLATED FUNC …`, `PROJECT_ENTRY_INVALID`); `tour` #12/13 were
   fragments. None was a broken page. A block is now a standalone program only
   if it also defines `main`; IMPORT-led blocks without one are **counted as
   fragments and reported, never compiled**.

One of the 18 was real: **`flow` #9** (`mfb man flow pipeline`, "A simple
two-stage pipeline") declares its helper `FUNCTION isEven … END FUNCTION`.
MFBASIC spells it `FUNC … END FUNC`; the example did not compile. Fixed in
`src/docs/man/flow/pipeline.md`.

Final run, every topic: `variable` 10/10, `flow` 10/10, `tour` 1/1, `types`
1/1, `lambda` 1/1 standalone programs build and run; fragments counted —
`tour` 12, `types` 1, `errors` 1; `link`, `optimizations`, `tooling`, `unicode`
have no standalone program. Each run prints its fence / program / fragment
split.

### C-3 — carve-out 4, and why the `tour` pages get none

The whole-surface memory sweep had 109 unclassified hits, all in this letter's
guide topics. Every sentence describing **MFBASIC** was rewritten to keep its
fact. Two groups needed a classification decision rather than a substitution:

- **`link`'s C-ABI type rows** (`CPtr`: "Opaque native pointer", `CString`:
  "Null-terminated UTF-8 pointer"). A binding author writes C types, and those
  rows would be false without the word. Carve-out 4, narrowed to exactly those
  two table rows, and recorded in `.ai/man-content.md` §9.2a. Every other hit on
  `link` — "ownership rules", "consumes", the `FREE` and loader prose — was
  rewritten.
- **The `tour` comparison pages** name other languages' memory models to
  contrast with them. **No carve-out**: each hit was rephrased in that
  language's own terms. A rule keyed on "this line is about another language"
  cannot be made precise enough to stop it hiding a sentence about MFBASIC.

Carve-out 3 was also found **silently disarmed**: its end anchor was the
rendered heading "Always-on lowering (Level 0)", which this letter's own
`optimizations` rewrite renamed to "Always-on rewrites". The census now accepts
both spellings, and carve-out 3 applies to `--memory-scope` as well (the catalog
rows say "lifetimes", "frees" and "allocation").

### C-4 — 38 invisible citation markers

`.ai/man-content.md` §3 bans `[[path:Symbol]]` markers on a man page, and the
renderer strips them, so **no rendered sweep has ever seen one.**
`grep -rn '\[\[\(src\|build\.rs\|repository\)' src/docs/man --include='*.md'`
found them in `link` (13), `types` subpages (20: `numeric` 14, `set` 4, `list`
1, `map` 1), `types` overview (4) and `lambda` (1). All removed; the check is
now recorded in `.ai/man-content.md` §9.2b.

### C-5 — Codex quota ran out mid-batch; three Phase 2 units re-dispatched

The Phase 2 batch passed its quota probe (A C-11) at dispatch. Five units then
ran and three — `term`, `astrings`, `crypto` — exited `FAILED` in 3 s with
"You've hit your usage limit … try again at Sep 13th, 2026 3:16 AM" in their
`.log` files. The probe row guards the *start* of a batch, not its end, so a
batch can straddle the limit. The harness recorded each failure honestly
(`manifest.tsv` `FAILED`, `--reconcile` `unaccounted=3`), so nothing was lost:
after the reset the probe answered `PONG` and only those three units were
re-dispatched, from `planning/plan-125-units/B-phase2-retry.txt` under the same
letter. `--reconcile` reads each unit's last manifest row. The user stopped
the session at the limit and resumed it after the reset.

### C-6 — Phase 4 had no prompt and no artifact; both added

§3.2 specifies four dimension-scoped runs over a condensed artifact, and plan-A
§3.4 says B runs them, but plan-A §5 defined only the eight iteration and
final-lens prompts. `man-final-lens.txt` is the nearest one, and it is the wrong
instrument: its six lenses read the whole 61,000-line manual, while §3.2's four
dimensions read only the overviews, `types` pages and topic overviews. Neither
`scripts/man-manual.sh` nor anything else produced that condensed artifact.

Added:
- `planning/plan-125-prompts/man-consistency.txt`: the four §3.2 dimensions
  (`vocabulary`, `overview-shape`, `guide-package`, `handles`) as
  `{{TARGET}}`, same output shape as the final-lens prompt.
- `scripts/plan-125-prompts-sync.sh`: now keeps nine prompts, the new one as
  §5.9.
- `scripts/man-manual.sh --condensed`: the 31 package overviews, each followed by
  its `types` page where one exists, then the 10 topic overviews. Measured:
  7,205 lines, 62 `═` pages (31 + 21 + 10; §2 said 20 types pages, and there
  are 21). Two runs are byte-identical, with nothing on stderr.
  - The first version listed packages with `ls`, the same way `--count` does,
    and printed `unknown package 'float_result.rs'`: a loose `.rs` file inside
    `src/codegen/builtins`.
  - `--condensed` now lists directories only. `--count` hides the same error
    behind `2>/dev/null`, and the stray file adds 0 to its page count.

The iteration prompts are unchanged, so B's findings remain comparable with
every other letter's.

## Summary

The yield concentrates in Phase 1: `canvas` and the nine guide topics are the
only material in the man surface that no independent reviewer has ever seen.
Phase 4 is the plan's only whole-surface look before iteration 2 fragments it,
so a vocabulary decision deferred out of Phase 4 costs 590 pages of drift.
