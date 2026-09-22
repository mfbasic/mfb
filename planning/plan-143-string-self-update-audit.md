# plan-143: Audit which `String` self-updates the compiler performs in place

Last updated: 2026-09-20
Effort: medium (1h–2h)

A research spike that **changes no code**, the `String` counterpart of plan-141.
It reads the compiler and records, for every builtin overload with a `String` (or
`AttributedString`) self-update form `s = pkg::f(s, …)`, and for `s = s & t`,
whether a `MUT` binding updating itself is mutated **in place** or **rebuilt as a
copy**, at each non-record binding site. Every verdict cites the deciding code and
is cross-checked with `mfb build --ncode`. The output is one findings file; the fix
plan is written from it.

The rule measured against is the same as plan-141's: **a `MUT` updating itself
mutates in place, with no copy.** This plan also classifies *whether an in-place
form can exist at all* for each function, because `String` results change length
in ways collection results mostly do not.

Why a separate audit (plan-142-A Open Decision 2, resolved by the user
2026-09-20): a `String` is a tight `[len:8][bytes][NUL]` block, can be a read-only
data constant, and grows in place today only through the self-concat capacity
shadow (plan-121-G). That is a different representation problem from collection
blocks, so plan-142 keeps only `s = s & t` and this audit maps the rest.

References:

- `planning/plan-141-findings/inplace-audit.md` — the method, the site legend,
  the marker technique (Appendix C), and §1b's `String` self-concat row (S1 `y`,
  S2 `n (StoreGlobal)`, S9 `n (G1)`).
- `src/codegen/collection/assign/builder_inplace_assign.rs:1714`
  (`try_inplace_concat_assign`) and `:1757` (`lower_string_self_append_one`);
  `builder_control.rs:2291-2345` (`string_capacity_slot_for`,
  `prescan_string_self_appends`).
- `planning/completed/plan-121-F-string-element-writes.md`,
  `planning/completed/plan-121-G-string-accumulator-folds.md` — prior `String`
  in-place work.
- plan-142 (`planning/plan-142-A-self-update-guard-and-seam.md`) — the collection
  fix plan whose seam (`SELF_UPDATE_TABLE`, four sites) the `String` fix plan will
  likely extend; this audit records where that seam would and would not fit.
- `mfb man strings`, `astrings`, `encoding`, `fs`, `os`, `io`, `net`, `regex` —
  the function lists.

## Prerequisites

None: this plan reads code and writes one findings file.

| Must be true | Command | Status |
|---|---|---|
| The release compiler exists, for the `--ncode` cross-checks | `ls target/release/mfb` → exists | MET (2026-09-21, re-run by /follow-plan in the P-143 worktree: `cargo build --release` exit 0, `target/release/mfb` listed) |

## 1. Goal

- `planning/plan-143-findings/string-self-update-audit.md` exists and holds, for
  **every overload with a `String`/`AttributedString` self-update form** and for
  `s = s & t`, a verdict per binding site (`y`, `n (<gate or path>)`, `n/a` with a
  reason), each `y`/`n` citing the deciding code as `file:symbol` and confirmed by an
  `--ncode` probe; plus a **form** column classifying whether an in-place form can
  exist (§4).

### Non-goals (explicit constraints)

- **No code, test, golden, spec or man-page changes.** Only the findings file and
  this plan's checkboxes.
- **Records, `RES`, and `STATE` are out of scope** (the user's scope is non-record
  self-updates). A `String` record field is listed only as "excluded".
- **Not timing-based.** Verdicts come from reading code; the one cross-check is the
  compiler's own `--ncode` output.
- No fix design beyond the form classification.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| Builtin packages / overloads | 42 / 828 | plan-142-A Appendix census script → `packages 42 overloads 828` |
| Overloads whose first parameter is `String`/`AttributedString` and whose return type is the same, literally | 44: `strings` 20, `encoding` 8, `fs` 6, `astrings` 4, `os` 3, `io` 1, `net` 1, `regex` 1 | the same script with the type test changed to `ft in ("String", "AttributedString", "astrings::AttributedString") and ft == ret` (Appendix) → `uniq -c` per package |
| Generic overloads whose result can be the first argument's `String` type (`Var`/`Arg(n)`) | 0 | `grep -rn 'ParameterType::Var\|ParameterType::Arg\|ParameterType::var(' src/codegen/builtins/{strings,astrings,encoding,regex,fs,os,io,net}` → no output; `census.py generic` (findings Appendix A) → 25 candidates across all 42 packages, of which the 11 true generics all take a `List`/`Map`/`Thread` first |
| Tier-B `AttributedString` overloads of `strings::` transforms (prose-documented, invisible to the man census) | 19 | `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs:332` → 19 entries; each compiles as `a = strings::f(a, …)` (probe `/tmp/plan-143-probes/try`) |
| Unqualified builtin with a `String` self-update form | 1: `toString(s)` | `general::resolve_call` `TO_STRING` arm accepts `ParameterType::String` (`src/codegen/builtins/general/mod.rs:393`) |
| §1 rows | 63 = 44 + 19 | `python3 census.py rows \| wc -l` → 63 |
| Operator self-update forms | 2: `s = s & t` (and chains) on `String`, `a = a & b` on `AttributedString` | plan-141 findings §1b; `mfb man astrings` ("`a & b` joins two `AttributedString` values") |

The 44, by name (`strings`): `caseFold graphemeAt left lower mid normalizeNfc padLeft
padLeftToWidth padRight padRightToWidth repeat replace right stripPrefix stripSuffix
trim trimChars trimEnd trimStart upper`; (`encoding`): `formUrlDecode formUrlEncode
htmlEscape htmlUnescape percentDecode percentEncode punycodeDecode punycodeEncode`;
(`fs`): `canonicalPath pathBaseName pathDirName pathExtension pathNormalize readText`;
(`astrings`): `addAttribute clearAttributes`×2 `removeAttribute`; (`os`): `getEnv
getEnvOr resourcePath`; `io::input`, `net::percentDecode`, `regex::replace`.

### What is already known (to be re-verified, not assumed)

- `s = s & t` is in place at S1 only (plan-141 §1b): `try_inplace_concat_assign`,
  gated by a capacity shadow slot from `prescan_string_self_appends` (G19), the
  chain shape (G20), and no later operand reading `s` (G21). S2 is `StoreGlobal`
  (no arm); S9 is G1.
- No other `String` self-update has a recogniser: none of the 19 non-`STATE` arms
  matches a `strings::` builtin (plan-141 Appendix B.3). Phase 2 confirms it.
- A `String` literal can live in read-only data (`call_returns_rodata_string`,
  `static_string_value` in `value_needs_owning_copy`, `builder_values.rs:907-912`):
  a binding may hold a pointer it must never write through. Phase 2 records how a
  binding's block becomes writable (copy on bind, or not).

## 3. Design Overview

The same table fill as plan-141 §3: rows are overloads, columns are sites; each
cell = the lowering path the statement takes, the recogniser on it (if any), and
the first declining gate in code order. One `--ncode` build covers every cell,
read with plan-141's marker method (arm stack-slot names unique in `src/`,
`store_global_*` for the global path).

Sites (plan-141's legend, non-record subset):

| Site | Meaning |
|---|---|
| S1 local | `MUT s` in a FUNC/SUB body |
| S2 global | `MUT s` at module level |
| S7 loop-live | n/a for every row: a `String` is not a `FOR EACH` iterable (recorded once, with the evidence) |
| S9 captured | `MUT s` assigned inside a non-escaping `forEach` `LAMBDA` (by-ref capture) |

## 4. The form column

For each function, one of:

| Form | Meaning | Example |
|---|---|---|
| `shrink` | result is a prefix/suffix/substring of `s`'s bytes, or `s` with bytes removed | `trim`, `left`, `right`, `mid`, `stripPrefix` |
| `same-len` | result has exactly `s`'s byte length for every input | ASCII-only case maps, if the code proves them |
| `rewrite` | result derived from `s` with length that can change either way | `upper`/`lower` over Unicode, `replace`, `normalizeNfc`, `htmlEscape` |
| `grow` | result is `s` plus added bytes | `padLeft`, `padRight`, `repeat` |
| `not-derived` | result is not a function of `s`'s bytes | `fs::readText(path)`, `os::getEnv(name)`, `io::input(prompt)` |

`not-derived` rows are the `Exempt` analogue of plan-142-E: the audit records
whether `s` is copied (it should only be read). Each form verdict cites the
function's lowering (the code, not its man page).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. `- [~]` partial. Moot tasks struck through with evidence,
> never deleted. **An unticked box means NOT DONE.**

### Phase 1 — Row census

- [x] Measure the generic `String` self-update overloads (the UNMEASURED row):
      read the registry signatures of `strings`, `astrings`, `encoding` and `regex`
      for `Var`/`Arg(n)` returns that can equal the first parameter's type; record
      the count and names in Measured populations. → 0 (grep of the eight
      packages' registries for `Var`/`Arg` → no output; `census.py generic` → no
      candidate takes a `String` first). Found instead: 19 prose-documented
      `AttributedString` overloads and `toString(s)` (Correction 1).
- [x] Create `planning/plan-143-findings/string-self-update-audit.md` with the
      sections: site legend, §1 overloads table
      (`| function definition | form | S1 | S2 | S7 | S9 | evidence |`), §2 the
      `&` operator row(s), §3 summary, appendices (census script, recogniser/lowering
      map, probes).
- [x] Generate one row per overload (signature verbatim from `mfb man <pkg> <f>`)
      with the census script kept in the appendix. → `census.py rows | wc -l` → 63;
      the script is findings Appendix A.

Acceptance: every overload has a row.
  Check: `grep -cE '^\| `(strings|astrings|encoding|fs|os|io|net|regex)::' planning/plan-143-findings/string-self-update-audit.md`
  → 44 + the Phase 1 generic count (est. 1 min). Corrected (Correction 1): → 63
  (44 literal + 0 generic + 19 Tier-B `AttributedString`). Measured: `63`.
Commit: 77ea4c292

### Phase 2 — Map the lowering paths

- [x] For each package, record how its `String` builtins lower (`Body` kind:
      `abi_inline`, `Intrinsic`, `Mfb`, `Rewrite`, `AbiFunction`) and whether the
      lowering allocates a fresh block for the result (cite the allocation).
      → findings Appendix B.2, one row per §1 row (`grep -o 'body: Body::…'` over
      each `func_*.rs`); every result is a fresh block, except `fs::pathDirName`'s
      rodata `.`/`/` (finding F3).
- [x] Confirm no recogniser matches any of these builtins (grep the arms' builtin
      names against the list), and record where a `String` self-update statement is
      dispatched at S1, S2, S9 (the `NirOp::Assign` chain, `StoreGlobal`, the by-ref
      fallback). → Appendix B.1: the arm-name grep meets the §1 names only in
      `mid`/`replace`, which decline at G10; 22 `abi_inline`/`Intrinsic` rows build
      an S2/S9 site (`su_global_block`/`su_ref_block` in 22/22 probes each), 41 never do.
- [x] Record the `String` representation facts the form column depends on: the
      tight block layout, the capacity shadow (who allocates it, when it resets —
      `builder_control.rs:2285-2345`), the read-only-data literal case (can a `MUT s`
      binding ever hold a rodata pointer at the moment of a self-update?), and how a
      `String` passed to a callee is borrowed.

Acceptance: the appendix lists every package's lowering kind for its rows, and the
three representation facts, each with a citation.
  Check: `grep -c '^| ' <appendix lowering table>` → one row per function (est. 2 min).
  Measured: `awk '/^### B.2/,/^### B.3/' … | grep -c '^| r'` → 63 (one per §1 row;
  `clearAttributes`' two overloads have different bodies, so rows, not functions);
  B.3 holds four representation facts, each cited.
Commit: db25ecfc7

### Phase 3 — Fill the table

- [x] For each row: the form (§4) and S1/S2/S7/S9 verdicts with the first
      declining gate or path. → 63 rows filled by `table.py rows` (findings C.5):
      S1/S2/S9 `n (no arm)` except `strings::mid`/`replace` (`String`) `n (G10)`;
      S7 `n/a (not iterable)` everywhere.
- [x] One `--ncode` build in `/tmp/plan-143-probes/` with one SUB per (row, site),
      read with plan-141's `markers.py` (arm marker slots, `store_global_*`); record
      the marker line next to each verdict. Reading vs dump disagreements: record
      both, the dump wins. → `str/`: 201 SUBs (67 cases × S1/S2/S9), built
      without diagnostics; marker line quoted in every evidence cell; `table.py`
      checks each reading against the dump and found no disagreement (C.2).
- [x] For `not-derived` rows, record whether `s` is copied (read the lowering; a
      copy is a finding). → 5 rows: `io::input` only reads `s`; `fs::readText`,
      `fs::canonicalPath`, `os::getEnv`, `os::getEnvOr` copy it into a
      helper-scratch C string (finding F4). Runtime sweep (C.3) added:
      198/201 pass, the 3 failures are F1.

Acceptance: no empty cell in the table.
  Check: `grep -E '^\| `(strings|astrings|encoding|fs|os|io|net|regex)::' planning/plan-143-findings/string-self-update-audit.md | grep -cE '\|\s*\|'`
  → 0 (est. 1 min). Measured: `0` (and 63 rows).
Commit: —

### Phase 4 — The `&` operator and the summary

- [ ] §2 rows for `s = s & t` and the chain form at S1/S2/S7/S9 (re-verify
      plan-141 §1b against the code at this commit).
- [ ] Summary: per-site and per-form counts (counted from the tables by a script
      kept in the appendix), the deciding paths, the functions whose form makes an
      in-place fix possible, and every place plan-142's seam would or would not fit
      a `String` arm (a `String` block is not a collection block: no
      `CollectionTypeLayout`, so G10 declines by construction — record it).
- [ ] List any finding that contradicts `.ai/collections.md`, plan-121-F/G, or the
      `mfb spec` memory section, for the fix plan to correct.

Acceptance: summary counts equal the table's cell count.
  Check: the appendix's count script prints the same total as rows × sites (est. 3 min).
Commit: —

## Validation Plan

- Tests: none (no code).
- Coverage check: the Phase 1 row count against the census; every row has a probe.
- Runtime proof: n/a; `--ncode` cross-checks instead.
- Doc sync: none; contradictions are listed for the fix plan.
- Final gate: `git status --short` shows only `planning/plan-143-*` paths changed
  (est. 1 min).

## Open Decisions

- **Include `AttributedString`?** Recommended: yes (4 overloads), since a
  `MUT a AS AttributedString` self-update is the same question; alternative:
  `String` only.
- **`not-derived` rows in the fix plan's guard.** Recommended: record them here as
  the `Exempt` analogue, so the fix plan's census can list them with a proof that
  `s` is only read; alternative: leave them out of the audit table.

## Corrections

1. **The row population was 44, it is 63.** The literal man census cannot see the
   19 Tier-B `AttributedString` overloads of `strings::` transforms: they are
   typed by `strings::resolve_return_type` (`src/codegen/builtins/strings/mod.rs`)
   and documented only in prose, so `mfb man strings trim` shows one `String`
   declaration. `a = strings::trim(a)` on a `MUT a AS AttributedString` compiles
   (probe `/tmp/plan-143-probes/try`), so each is a self-update form in scope
   under Open Decision 1 (recommended option, include `AttributedString`). The
   generic count the plan left UNMEASURED is 0. `toString(s)` (unqualified,
   `general::resolve_call` accepts `String`) and `a = a & b` on `AttributedString`
   are further forms; they go in findings §2 beside `s = s & t`, since they are
   not package overloads. Phase 1's acceptance number is corrected to 63.
2. **plan-142 has landed since this plan was written** (`planning/completed/plan-142-*`),
   so plan-141 §1b's `s = s & t` verdicts (S2 `n (StoreGlobal)`) and the
   references' line numbers are stale: `StoreGlobal` now dispatches the arms
   (`builder_control.rs:1076-1105`), and `try_inplace_concat_assign` is at
   `builder_inplace_assign.rs:1609`, `string_capacity_slot_for` at
   `builder_control.rs:2351`, `prescan_string_self_appends` at `:2374`. Every
   verdict here is read at `efdb54bb7`.
3. **Two memory-safety bugs surfaced by the audit** (findings §3.2 F1, F3), fixed
   under bug-667 (whose producer audit they belong to): `s = toString(s)` frees the
   block it stores, at S1, S2 and S9 (`toString(<String>)` returns its argument's
   block); `fs::pathDirName` returns a rodata `.`/`/` that the owner later frees
   (SIGBUS). Found by an added runtime sweep (Appendix C.3) — not a plan task, but
   the plan's "a copy is a finding" check for `not-derived` rows needed the
   runtime view to be trusted. No code changes here: the fix lands from bug-667's
   own worktree.

## Summary

A read-only table fill over 44+ `String` overloads and the `&` operator at four
non-record sites, with a form column saying whether an in-place fix can exist. The
risk is again a false `y`; the new wrinkle is that many `String` results change
length, which the form column makes explicit before any fix is designed.

## Appendix — census command

plan-142-A's Appendix script, with the type test replaced by:

```python
if ft in ("String", "AttributedString", "astrings::AttributedString") and ft == ret:
    hits.append(s)
```

Run 2026-09-20 against `target/release/mfb` built from `b6a10efbc`: 44 hits.
