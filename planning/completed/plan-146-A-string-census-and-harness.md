# plan-146-A: The `String` self-update census and harness

Last updated: 2026-09-21
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing (the Prerequisites below gate the whole of plan-146)

## plan-146 as a whole

**Goal of the whole plan:** every self-update `s = f(s, …)` of a `MUT` binding
whose value is a `String`, for a builtin `f` whose result is derived from `s`,
mutates `s`'s existing block in place: no copy of `s` and no fresh result block.
It must hold at the three non-record sites plan-143 audited: a function local
(S1), a module-level global (S2), and a `MUT` captured by reference in a
non-escaping `collections::forEach` lambda (S9). `s = s & t`, in place at S1 and S2
today, gains S9. A `String` builtin whose result is not derived from `s` gets an
`Exempt` row that proves `s` is only read, not copied. The guard plan-142 built for
collections extends to `String`: a new builtin with a `String` self-update form
and no arm or proven exemption fails the suite.

"In place" is measured the way plan-142 measured it. Run the statement `N` times
and `2N` times under `mfb build --debug` and sum `arena.<k>.alloc_calls`
(plan-142-A Correction A4). The bound is `count(2N) − count(N) < N/8`. A copying
lowering allocates the result block every statement, so it fails the bound.

S7 does not exist for a `String`: `FOR EACH` over a `String` is a type error
(findings Appendix B.1, `TYPE_FOR_EACH_REQUIRES_COLLECTION`).

The facts this plan starts from are plan-143's findings,
`planning/plan-143-findings/string-self-update-audit.md`:

- **§3.1:** of 67 `String`/`AttributedString` self-update forms, 1 is in place:
  `s = s & t` (and its chain) at S1 and S2. Every builtin self-update rebuilds `s`
  at every site.
- **§3.4 (the form column):** shrink 22, grow 11, same-len 4, rewrite 21,
  not-derived 5 (`table.py count`, findings §3.3).
- **§3.5:** what fits plan-142's seam and what does not.
- **§3.2:** F1 and F3 (use-after-free, SIGBUS) are fixed by bug-667. F4 (four
  helpers copy `s` into a C string) is still open.

| Letter | What it lands | Rows | Findings it closes | Effort |
|---|---|---|---|---|
| **A** | the census covers `String`; harness `String` lines with `pending:`/`deferred:` expectations | 40 `String` + `toString` + 24 deferred | §3.5 census bullet | large |
| **B** | the `String` seam: a `String` resolver, one capacity-shadow rule for every `String` arm target (byte-identical refactor), the call-target spellings, and the first arm, `s = toString(s)` | 1 | §3.5 G10, `self_update_builtin`, shadow bullets | large |
| **C** | shrink arms: `left right mid stripPrefix stripSuffix trim trimStart trimEnd trimChars graphemeAt`, `fs::pathBaseName pathDirName pathExtension` | 13 | F2 (part) | large |
| **D** | grow arms: `padLeft padRight padLeftToWidth padRightToWidth repeat`, `os::resourcePath` | 6 | F2 (part) | large |
| **E** | rewrite arms for the native rewrites: `upper lower caseFold normalizeNfc`, `strings::replace`, `fs::pathNormalize` | 6 | F2 (part) | large |
| **F** | `Exempt` rows, each proven copy-free: 5 not-derived (fixing F4 first) and 10 MFBASIC-body rewrites (8 `encoding`, `net::percentDecode`, `regex::replace`) | 15 | F4 | large |
| **G** | S9: a capacity shadow the lambda shares with its creator; every `String` arm and `&` fire at S9 | all arms | §3.5 S9 bullet, §3.6 item 1, O2 | large |
| **H** | lock the guard (no `pending:` left), sync the docs, run the full gate | — | §3.6 | medium |

Letter order is implementation order. A is tests only. B's first phase is a
byte-identical refactor. C–E add arms at S1 and S2: the seam already builds S2
sites, and B gives globals the same shadow rule as locals. F changes helpers that
only read `s`. G opens the one new aliasing surface: a callback holding the
parent's block and its shadow. So it comes last among the code letters, behind
the harness A–F built. plan-142 used the same order.

References:

- `planning/plan-143-findings/string-self-update-audit.md`: §1 and §2 cell tables,
  §3.2 findings, §3.4 forms, §3.5 seam fit, §3.6 doc contradictions, Appendix B.1
  (dispatch per site), B.2 (lowering per row), B.3 (the four representation facts),
  Appendix C (probes, `markers.py`, `table.py`).
- `planning/completed/plan-142-A-self-update-guard-and-seam.md`: the table, the
  seam, the harness, the failure-atomicity rule, Corrections A4–A6 (what the
  harness counts; an arm may not allocate per statement).
- `planning/completed/plan-142-G-lambda-capture.md` Correction G1 and
  `planning/completed/plan-142-H-globals.md`: why `&` has no S9 today, and the
  global shadow.
- `src/codegen/collection/assign/self_update.rs`: `SELF_UPDATE_ARMS` `:150`,
  `self_update_builtin` `:214`, `try_inplace_self_update` `:234`,
  `add_global_string_capacities` `:285`, the scratch `:356-660`, `SelfUpdate`
  `:663`, `SELF_UPDATE_TABLE` `:796`, `Site`/`ENABLED_SITES` `:1287`/`:1303`, the
  tests `:1374`.
- `src/codegen/collection/assign/builder_inplace_assign.rs`:
  `try_inplace_concat_assign` `:1609`, `lower_string_self_append_one` `:1688`.
- `src/codegen/engine/control/builder_control.rs`: `NirOp::StoreGlobal` `:1076`,
  `NirOp::Assign` `:1207`, `string_capacity_slot_for` `:2366`,
  `reset_string_capacity_shadow` `:2377`, `prescan_string_self_appends` `:2389`,
  `string_self_append_operands_of` `:2992`.
- `src/docs/spec/memory/03_heap-values.md` ("Standalone String"),
  `04_arenas.md` (free sizes come from the compiler), `05_collections.md:492`
  ("Self-updates").
- `.ai/testing-gates.md:10` (per-phase gate), `:809` (a codegen-inspection test
  must be proven RED by reverting the fix), `.ai/compiler.md:85-86` (acceptance
  plus an execution test).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-142 is complete and archived | `ls planning/completed/plan-142-I-lock-and-docs.md` → exists | MET (2026-09-22, re-run at `549506901`) |
| plan-143 is complete and its findings exist | `ls planning/completed/plan-143-string-self-update-audit.md planning/plan-143-findings/string-self-update-audit.md` → both exist | MET (2026-09-22) |
| bug-667 fixed: `toString(<String>)`, `fs::pathDirName` and a default global `String` hand an owning store a block it owns (findings F1, F3) | `ls bugs/completed/bug-667-*.md` → exists, **and** `scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/general/tostring_string_owning_store'` and `… 'rt-behavior/fs/pathdirname_constant_owned'` → pass | MET (2026-09-22: the file exists; both fixtures `acceptance tests passed (1 test(s) ran)` in the P-146 worktree) |
| The release compiler exists for `--ncode` probes | `ls target/release/mfb` → exists | MET (2026-09-22: built in the P-146 worktree) |

Why bug-667 gates the whole plan: every arm here writes into `s`'s block. That is
sound only if `s` owns its block. F1 (`s = toString(s)` stored the block it freed)
and F3 (`pathDirName` returned read-only data) were the two producers that broke
that ownership (findings Appendix B.3 fact 3). With them fixed, a `MUT` `String`
binding never holds a block it does not own at a self-update. The concat arm has
relied on the same rule since plan-121-G.

plan-145 (record and `STATE` fields) is **not** a prerequisite and does not depend
on this plan. Both edit `self_update.rs`, `rt_inplace_self_update.rs` and
`cases.tsv`, so run them one after the other, not in parallel worktrees. Every
`String` arm here declines any destination other than `Direct`, `Global` or `Ref`
(B's resolver, gate `G-string-dest`). So an arm cannot fire at a field site,
whichever plan lands first.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. **If you stop, report the current status of *all*
> prerequisites.**

## 1. Goal (this letter)

- The unit census (`self_update_census_covers_every_registry_overload`) counts
  `String` overloads: `registry::self_update_shaped` accepts a first parameter of
  type `String` (or `AttributedString`) whose return type is the same type. It
  fails until each of the 44 literal rows has a `SELF_UPDATE_TABLE` row.
- A second census source, `TIER_B_TRANSFORMS`, requires a row for each of the 19
  prose-documented `AttributedString` overloads of `strings::` transforms. They
  are not registry overloads (findings Correction 1).
- The black-box census (`tests/guards/inplace_self_update_census.rs`) accepts
  `String`/`AttributedString` first parameters too. Each such `mfb man` signature
  needs a `cases.tsv` line.
- `SelfUpdate` gains `Pending(&'static str)` (the letter that lands the arm) and
  `Deferred(&'static str)` (the plan that owns it). Every new row is `Pending` or
  `Deferred`. Letter H deletes `Pending` again. plan-142-I deleted it, so this is a
  deliberate, temporary reopening, and H closes it.
- `cases.tsv` gains one line per new row. The harness accepts two more statuses:
  - `pending:<letter>`: the statement must still copy (`count(2N) − count(N) ≥ N`),
    so the letter that lands the arm has to flip the line;
  - `deferred:<tag>`: the same assertion, for rows another plan owns.

### Non-goals (explicit constraints)

- No codegen change. The only behavioral change is to tests.
- No change to the 79 existing `cases.tsv` lines or their expectations
  (`wc -l tests/runtime/inplace_self_update/cases.tsv` → 79).
- No `mfb man` change.
- `AttributedString` arms (Open Decision 1) and `String` record/`STATE` fields
  (plan-145-A Open Decision 1) are out of plan-146.

## 2. Current State

- `registry::self_update_shaped` (`src/codegen/registry/mod.rs:2896`, `#[cfg(test)]`)
  accepts only a first parameter that is, or binds to, `List`/`Map`/`Set` or a type
  variable. A `String` row is never required.
- The black-box census's `self_update_shaped` (`tests/guards/inplace_self_update_census.rs:238`)
  parses `List`/`Set`/`Map` only (`Ty::List | Ty::Set | Ty::Map`, `:247`).
- `SelfUpdate` has `Arm` and `Exempt` (`self_update.rs:663`). Operator rows are
  admitted by the tests' `OPERATORS = ["&"]` (`:1380`).
- The harness statuses are `arm` and `exempt`. Any other status panics
  (plan-142-I Phase 1; `cases.tsv` header). `Site::applies` skips S7 and S9 for a
  `String` case (`rt_inplace_self_update.rs:224`).
- `Probe::source` returns `None` for a `String` probe at `ForEach`/`Lambda`
  (`self_update.rs`, the `Site::ForEach | Site::Lambda if self.ty == "String"` arm).

### Measured populations

| What | Count | Command |
|---|---|---|
| Literal `String`/`AttributedString` self-update overloads | 44: `strings` 20, `encoding` 8, `fs` 6, `astrings` 4, `os` 3, `io` 1, `net` 1, `regex` 1 | plan-143 Appendix census (findings Appendix A) |
| …of which `String` | 40 (44 − the 4 `astrings`) | findings §1 rows whose first parameter is `String` |
| Tier-B `AttributedString` overloads | 19 | `TIER_B_TRANSFORMS` (`src/codegen/builtins/strings/mod.rs:332`) → 19 entries |
| Non-registry forms | 3: `s = s & t` (already a row), `s = toString(s)`, `a = a & b` on `AttributedString` | findings §2 |
| `String` rows per letter | C 13, D 6, E 6, F 15 (5 not-derived + 10 MFBASIC-body rewrites) = 40 | findings §3.4 and Appendix B.2 body kinds; the 40 split in the table above |
| Deferred rows (Open Decision 1) | 24 = 4 `astrings` + 19 Tier-B + `a & b` | same |
| New `cases.tsv` lines | 65 = 40 + `toString` + 24 | the two rows above |
| Harness cost per (line, site) pair | 2.7 s | plan-142-I Phase 1: 254 pairs in 679.52 s |

## 3. Design

**The census rule.** Extend `self_update_shaped` with one more accepted first type:
`ParameterType::String`, or the `astrings::AttributedString` record, when the
return type is the same type. The black-box twin gets the same rule. Neither rule
needs unification for these types: the findings found no generic overload whose
result can be the first argument's `String` type (findings Measured populations,
the generic census → 0).

**The Tier-B source.** A unit test, `tier_b_transforms_have_rows`, reads
`TIER_B_TRANSFORMS` through a `pub(crate)` accessor in `strings/mod.rs` and
requires a row spelled `strings::<member>@AttributedString` for each entry. The
`@AttributedString` suffix keeps it apart from the `String` row of the same
function. The stale-row test admits exactly these spellings.

**Non-registry rows.** `OPERATORS` becomes `NON_REGISTRY = ["&", "toString",
"&@AttributedString"]`. The `&` row stays `Arm([Concat])`, `toString` is
`Pending("B")`, and `&@AttributedString` is `Deferred("attributed-string")`.

**Row kinds, per letter:**

| kind | rows |
|---|---|
| `Pending("B")` | `toString` |
| `Pending("C")` | the 13 shrink rows |
| `Pending("D")` | the 6 grow rows |
| `Pending("E")` | the 6 native rewrite rows |
| `Pending("F")` | the 15 rows letter F exempts |
| `Deferred("attributed-string")` | the 24 `AttributedString` forms |

Each row carries a probe (`ty: "String"`) for the matrix. The matrix skips
`Pending` and `Deferred` rows, as it skips `Exempt`.

**Harness lines.** One `cases.tsv` line per row. A statement that cannot repeat
`2N` times alone is paired with a restoring statement, as plan-142-A Correction A5
did for collections:

- `x = x & "ab" ; x = strings::left(x, 3)`;
- `x = strings::padRight(x, 8) ; x = strings::left(x, 3)`.

The restoring statement uses an arm that is already in place (`&`) or one the
same letter lands, so a line measures the row it names. Where that pairing is not
possible, the restoring step is `x = <literal>` and the line's bound is computed
against a control program that runs only the restoring step. The first such line
adds the control column; record it as a Correction.

`io::input` reads stdin, and the `fs`/`os` rows touch the host. Their lines set up
what they need in the program itself:

- `io::input`: the harness feeds `N` lines on stdin;
- `fs::readText` / `fs::canonicalPath`: a file the program writes first;
- `os::getEnv` / `getEnvOr`: a variable that `os::setEnv` sets first.

Letter F owns these lines' `exempt` status. A writes them as `pending:F`.

Rejected alternatives:

- **Leave `String` out of the census and list the rows by hand.** A new `strings::`
  function would slip past, which is the gap findings §3.5 names.
- **Census `AttributedString` only through `astrings`.** It would miss the 19 Tier-B
  overloads, which `mfb man` does not list (findings Correction 1).

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` partial. Moot tasks struck through with evidence, never
> deleted. **An unticked box means NOT DONE.**

### Phase 1: The census rule and the table rows

- [x] `registry::self_update_shaped`: accept a `String` or `AttributedString` first
      parameter whose return type is the same type. Run the census and record the
      new failure list here. It must name exactly the 44 literal rows (40 + 4
      `astrings`). Any other name is a finding to record in Corrections before
      continuing.
      `cargo test --bin mfb self_update_census_covers_every_registry_overload` →
      44 names: `astrings::{addAttribute, clearAttributes, removeAttribute, writeSpans}`,
      `encoding::{formUrlDecode, formUrlEncode, htmlEscape, htmlUnescape, percentDecode,
      percentEncode, punycodeDecode, punycodeEncode}`, `fs::{canonicalPath, pathBaseName,
      pathDirName, pathExtension, pathNormalize, readText}`, `io::input`,
      `net::percentDecode`, `os::{getEnv, getEnvOr, resourcePath}`, `regex::replace`,
      and the 20 `strings::` names. The 44 are *functions*; the plan's 44 are
      *overloads* (`clearAttributes` has two). `astrings::writeSpans` is the one
      unexpected name: Correction A1.
- [x] `SelfUpdate::Pending(&'static str)` and `SelfUpdate::Deferred(&'static str)`;
      the stale-row test requires a non-empty string for both.
- [x] 44 rows + 19 Tier-B rows + `toString` + `&@AttributedString`, with the kinds
      in §3 and one probe each. (43 function rows after Correction A1: 40 `String`
      + 3 `astrings`, whose `clearAttributes` row carries a probe per overload.)
- [x] `tier_b_transforms_have_rows`, and the `NON_REGISTRY` list.
- [x] RED proof (`.ai/testing-gates.md:809`): delete the `strings::trim` row and
      confirm the census fails naming it. Delete the `strings::trim@AttributedString`
      row and confirm the Tier-B test fails naming it. Restore both.
      Both deleted at once, `cargo test --bin mfb -- self_update_census_covers
      tier_b_transforms_have_rows` → `2 failed`:
      `self-update-shaped builtin(s) with no SELF_UPDATE_TABLE row … ["strings::trim"]`
      and `Tier-B AttributedString transform(s) with no SELF_UPDATE_TABLE row:
      ["strings::trim@AttributedString"]`. Restored.

Acceptance: `cargo test --bin mfb self_update` passes; both RED failure lines are
recorded here (est. 5 min).
Result: `cargo test --bin mfb self_update` → `test result: ok. 8 passed; 0 failed`
(86.07 s); the RED lines are on the last task above.
Commit: e0ac58d30

### Phase 2: The black-box census and the harness lines

- [x] `tests/guards/inplace_self_update_census.rs`: the `String`/`AttributedString`
      rule. Record the number of shaped signatures it reads. Expected: 63 + 44 = 107.
      Measured 126 = 63 + 44 + 19: the rule reads the expected 107, and the census
      also derives the 19 Tier-B forms from the man pages (Correction A3). The
      census passes, so the shaped set equals the `cases.tsv` lines holding `::`:
      `grep -v '^#' cases.tsv | cut -f1 | grep -c '::'` → 126.
- [x] `rt_inplace_self_update.rs`: the `pending:<letter>` and `deferred:<tag>`
      statuses (both assert `count(2N) − count(N) ≥ N`), the stdin feed for
      `io::input`, and the `cases.tsv` header text for the two statuses. Also
      added (Correction A4): automatic `IMPORT`s for a plain-site program
      (`prelude_for`, shared with the field sites), `len_of` (`len` has no
      `AttributedString` overload), and `AttributedString` runs at S1/S2 only.
- [x] `cases.tsv`: the 65 new lines. Plus 975 `field_expect.tsv` lines
      (Correction A4), from `field_expect_gen.py`, whose output keeps the old 964
      lines byte-identical (`diff <(head -964 new) old` → no output).
- [x] RED proof: mark the `strings::trim` line `arm` and confirm the bound fails
      naming it. Mark the `&` line `pending:B` and confirm the "still copies"
      assertion fails. Restore both. `MFB_SELF_UPDATE_SITES=Local,Global
      MFB_SELF_UPDATE_FILTER='strings::trim(value AS String)|& (value AS String'` →
      `4 of 4 case/site pair(s) failed`: `strings::trim(value AS String) AS String at
      Local: marked `arm`, but 2000 more runs allocated 4000 more blocks (4155 at
      N=2000, 8155 at 2N) — the statement copies` (and at Global), and `& (value AS
      String, other AS String) AS String at Local: marked `pending`, but 2000 more
      runs allocated only 1 more blocks (166 at N=2000, 167 at 2N) — the statement no
      longer copies; flip the line (owner: B)` (and at Global). Restored.
- [x] Added (Correction A4): a field-site sample of the new expectations —
      `MFB_SELF_UPDATE_SITES=S3,…,T8` (all 15) with `MFB_SELF_UPDATE_FILTER` =
      `strings::left(value AS String|io::input|os::getEnv(|strings::trim(value AS
      AttributedString|fs::canonicalPath` (5 lines, 75 pairs) → `test result: ok`
      (56.48 s).

Acceptance: `cargo test --test inplace_self_update_census` passes, and the harness
passes on the new lines:
`for f in strings:: encoding:: fs:: os:: io:: net:: regex:: astrings:: toString '&@'; do MFB_SELF_UPDATE_FILTER="$f" cargo test --test rt_inplace_self_update || break; done`
(est. 6 min: 65 lines × S1, S2 at 2.7 s per pair. This is the one run that checks
every new expectation against today's compiler. `strings::` also matches no
existing line: the 79 existing lines are `collections::`, `math::`, `compress::`,
`crypto::` and `&`). Record the pair count each filter ran here; they must sum to
130.
Result: `cargo test --test inplace_self_update_census` → `2 passed` (63.16 s). The
loop (`/tmp/p146_loop.sh Local,Global`, the filter loop above with
`MFB_SELF_UPDATE_SITES=Local,Global` — Correction A4) → every filter `test result:
ok`: `strings::` 43 lines (86 pairs, 73.26 s), `encoding::` 8 (16), `fs::` 6 (12),
`os::` 3 (6), `io::` 1 (2), `net::` 1 (2), `regex::` 1 (2), `astrings::` 4 (8),
`toString` 1 (2), `&@` 1 (2) — 138 pairs, 130 distinct: `strings::` also matches the
four `astrings::` lines (the substring), which the `astrings::` filter reran.
Commit: bca4ebacb

## Validation Plan

- Tests added: the `String` census rule (unit and black-box), the Tier-B census,
  65 harness lines, the two new statuses.
- Per-letter unit gate: `cargo test --bin mfb` (`.ai/testing-gates.md:10`).
- Doc sync: none in this letter (letter H).
- The full gate runs once, in letter H.

## Open Decisions

1. **`AttributedString` (24 forms: 4 `astrings`, 19 Tier-B, `a & b`).** Recommended:
   out of plan-146, rows `Deferred("attributed-string")`, harness lines asserting
   they still rebuild. An `AttributedString` is a record: its text is a `String`
   field and its spans are a `List` field. Every one of its forms is an MFBASIC
   helper that rebuilds the record (findings Appendix B.2 r01–r04, r45–r63, and
   §2's `#astrings_concat`). In place means a `String` arm at a record field plus
   an in-place span remap. That needs plan-145's field seam and this plan's
   `String` arms. A follow-up plan written after both land owns it, together with
   plan-145's `deferred:string` field lines. The Deferred rows and lines are its
   starting census.
   Alternative: land `AttributedString` here, after plan-145. That makes plan-145 a
   prerequisite of plan-146 and adds a letter.
   DECISION: recommended (Correction A2).
2. **The 10 MFBASIC-body rewrites** (`encoding::formUrlDecode formUrlEncode
   htmlEscape htmlUnescape percentDecode percentEncode punycodeDecode
   punycodeEncode`, `net::percentDecode`, `regex::replace`). Recommended: `Exempt`
   in letter F, each with a proof that the helper only reads `value` and streams
   its output into its own buffer. This is the same argument plan-142-E made for
   the `compress` codecs: the output is a new byte stream built by reading `x`,
   with no copy of `x` to avoid. Where a helper does copy `value`, F fixes that
   first, as plan-142-E did for `argon2id`/`shake256`.
   Alternative: native in-place arms for all 10. That means re-implementing eight
   codecs and a regex replace in native lowering, at least two more large letters,
   for functions rarely applied to a binding in a loop (in-tree uses:
   `examples/browser/display/src/lib.mfb:49-51`, three `regex::replace` lines, from
   the fixture grep in B's Measured populations).
   DECISION: recommended (Correction A2).
3. **S9 for `String`.** Recommended: letter G, a capacity shadow shared by the
   creator and the lambda through the closure environment, the way the
   self-update scratch is shared (`scratch_closure_captures`,
   `emit_publish_borrowed_scratch`). plan-142-G Correction G1 kept `&` out of S9
   because the lambda saw the parent's binding slot but not its shadow. Every grow
   and rewrite arm here has the same need, so the gap is no longer one row.
   Alternative: keep S9 as a recorded gate (`G1`) for every `String` arm, each with
   a harness line asserting the copy. That is plan-142's answer for `&`.
   DECISION: recommended (Correction A2).
4. **What an in-place shrink does with the freed bytes.** Recommended: keep them as
   spare capacity in the binding's shadow (`shadow += oldLen − newLen`), as the
   collection shrink arms keep `dataCapacity`. B gives every `String` arm target a
   shadow, so the drop still frees the block at its true size (findings B.3 fact 1).
   Alternative: return the tail to the arena. The allocator takes a caller-supplied
   size and keeps no chunk header (`04_arenas.md`, "Free-List Layout"), so freeing
   the 16-aligned tail looks legal, but nothing in-tree frees part of a chunk
   (UNVERIFIED), and it saves memory only for a binding shrunk and never grown
   again.
   DECISION: recommended (Correction A2).

## Corrections

- **A1 — the census names `astrings::writeSpans`, a function user source cannot
  call.** Phase 1's first census run listed 44 functions, one more than the 43
  functions behind the plan's 44 overloads: `astrings::writeSpans(value AS
  AttributedString, spans AS …) AS AttributedString`
  (`src/codegen/builtins/astrings/func_write_spans.rs`) is `internal_only: true` —
  "a native primitive the package's own injected companion calls that user source
  must never reach" (`RegistryFunction::internal_only`, `src/codegen/registry/mod.rs`),
  so it has no man page (`mfb man astrings writeSpans` → `unknown astrings
  function`) and no user program can write `a = astrings::writeSpans(a, …)`. The
  unit census now skips `internal_only` functions (`shaped_functions`); no
  existing row is `internal_only` (the census still passes with every plan-142 row).
  The black-box census reads `mfb man`, which never lists it. Row counts:
  `SELF_UPDATE_TABLE` gains 43 registry rows (40 `String`, 3 `astrings`), 19 Tier-B
  rows and 2 non-registry rows (`toString`, `&@AttributedString`) = 64 rows; the
  harness gains one line per overload, 65 (the `clearAttributes` row covers two).
- **A2 — the Open Decisions were blank; the plan runs on the recommended options.**
  Every letter (C's shadow arithmetic, F's `Exempt` set, G's shared shadow, the
  `Deferred` rows here) is written for the recommended option of each of Open
  Decisions 1–4, and `/follow-plan` executes the plan as written, so each
  `DECISION:` line now records "recommended". A user decision the other way is a
  re-plan, not a correction.
- **A3 — the black-box census reads the Tier-B forms too.** §3 gave the Tier-B
  overloads only a unit census (`TIER_B_TRANSFORMS`), since `mfb man` renders no
  overload for them. But each of their 19 pages says so in one sentence ("value may
  also be an astrings::AttributedString: it returns an AttributedString";
  `grep -rln 'may also be an `astrings::AttributedString`: it returns'
  src/codegen/builtins/strings/` → exactly the 19 `TIER_B_TRANSFORMS` members), so
  the black-box census derives each such function's `AttributedString` signature
  from its `String` one and requires a `cases.tsv` line for it, and asserts it found
  at least 19 (a guard against the wording changing). The Tier-B lines are spelled
  as those derived signatures (`strings::left(value AS AttributedString, count AS
  Integer) AS AttributedString`), not with the table's `@AttributedString` suffix;
  the two non-registry lines have no `::` (`toString(value AS String) AS String`,
  `&@AttributedString (value AS AttributedString, …)`), which the census skips as
  it skips `&`. Shaped count: 126, not 107.
- **A4 — every `cases.tsv` line also runs at the 15 field sites (plan-145).** The
  plan predates plan-145-A's field harness: `FieldCase::from_case` panics for a
  line with no `field_expect.tsv` expectation at every field site, so the 65 lines
  need 975 expectations. `field_expect_gen.py` now gives every line whose `x` is a
  `String` or an `AttributedString` `deferred:string` (plan-145-A Open Decision 1:
  no `String` arm serves a field; plan-146's arms are `FieldReach::None`), and
  `na:TYPE_FOR_EACH_REQUIRES_COLLECTION` at S7/T7 — the `&` line's existing rule,
  regenerated byte-identically. The acceptance loop above runs S1/S2 only
  (`MFB_SELF_UPDATE_SITES=Local,Global`), the 130 pairs this letter names; the 975
  field pairs are part of the full harness in letter H (and a sample ran here:
  Phase 2's last task). Other harness additions the lines needed: automatic
  `IMPORT`s for a plain-site program (`prelude_for`; programs whose packages are
  all in `PRELUDE` are byte-identical), `len_of` (`len` has no `AttributedString`
  overload: `TYPE_CALL_ARGUMENT_MISMATCH`, so `strings::byteLen`), and
  `Site::applies` keeping an `AttributedString` line to S1/S2 as a `String` one is.
  `io::input` gets `STDIN_LINES` empty lines (at EOF it raises `7-702-0003`), and
  its line starts from `""` so the prompt it echoes is empty.

## Summary

A extends plan-142's two censuses to `String` (and, as deferred rows,
`AttributedString`), adds a second census source for the Tier-B overloads `mfb man`
cannot see, and writes one harness line per row with an expectation that fails
the moment a letter changes more than it claims. It changes no codegen.
