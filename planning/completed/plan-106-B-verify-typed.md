# plan-106-B: ir::verify onto ParameterType (typed env, structural rules)

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-106-A (the lowering-side engines are typed; verify's oracle
must agree with what lowering now emits natively).

Retype `ir::verify`'s reconstructed type environment and inference onto
`ParameterType`: `infer_type -> Option<ParameterType>` (44 call sites), the
`String`-valued env stores (42 `HashMap<String, String>` occurrences —
`locals`/`globals`/`field_types`/`record_field_lists`/`FnSig.params`/`.returns`),
and the string helpers (`resource_base_type`, `parse_map`,
`read_only_record_type`, `is_defaultable`, the `usable_type` seam) become
structural. This deletes the ~30 `.name()` read-shims recorded as deliberate
residue in plan-102-B Phase 3 and closes that deferral.

See plan-106-A for the roadmap, the shared prerequisites, and the terminal
no-strings invariant this letter advances.

References:

- `src/ir/verify/mod.rs` — `TypeEnv` stores, `infer_type` (`:955`),
  `resource_base_type`/`parse_map`/`read_only_record_type`; `values.rs`,
  `compat.rs` (the compatibility algebra), `calls.rs`, `resources.rs`,
  `types.rs`, `link.rs`.
- `planning/completed/plan-102-B-typed-ir.md` §Phase 3 — the recorded deferral
  ("0 re-parse, render-only") this letter retires.
- The soundness rule at `verify/mod.rs:26-31`: verify must accept exactly what
  lowering emits — the byte-identical golden suite is the oracle.

## Prerequisites

See plan-106-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-106-A complete | `ir::lower::expression_type` returns `Option<ParameterType>`; A's boxes ticked | **MET** 2026-08-24 — `src/ir/lower.rs:1965` reads `fn expression_type(…) -> Option<ParameterType>`; A's three phases all ticked, commits `f20b96ca9` + `f3c81d81a` |

## 1. Goal

- `TypeEnv`'s stores are `ParameterType`-valued; `infer_type` returns
  `Option<ParameterType>`; the rule implementations compare structurally.
- The diagnostic **text, codes, and order are byte-identical** — every message
  that quotes a type renders `name()` at the `format!` site.
- ~~`rg -c '\.name\(\)' src/ir/verify/*.rs` drops from 30 to only
  message-formatting sites (each listed in the acceptance).~~ — **replaced by
  the grammar/environment census** (Correction 2): a raw `.name()` count cannot
  tell a render-out from a re-parse, and the retype *raises* it by moving each
  diagnostic's render to its own `format!`.
- `resource_base_type` (strip ` STATE `) and `parse_map` are replaced by
  structural equivalents (STATE handling per the RES/STATE sibling model —
  verify reconstructs from IR fields that already carry `ParameterType`).

### Non-goals (explicit constraints)

- No change to which programs are accepted/rejected — the full `*-invalid`
  diagnostic golden corpus is the gate, alongside byte-identity for accepted
  programs.
- The `RELOCATED_TO_IR_VERIFY` rule-split list and the dual-pass topology are
  untouched (C/D restructure the other side).
- The package-path hardening semantics (`Unknown` skips, PKG-02/PKG-03 caps)
  are behavior — preserve exactly (`ParameterType::Unknown` is the same
  sentinel, now structural).

## 2. Current State

Post-plan-102-B, verify reads typed IR fields but renders them into a string
env (the recorded deferral: "0 re-parse; rendering, not re-parsing"). All rule
logic — compatibility algebra in `compat.rs`, literal ranges in `values.rs`,
STATE agreement in `calls.rs` — compares strings.

### Measured populations

| What | Count | Command |
|---|---|---|
| `infer_type` call sites | 44 | `rg -c 'infer_type\(' src/ir/verify/ \| awk -F: '{s+=$2} END{print s}'` → 44 |
| `HashMap<String, String>` occurrences | 42 | `rg -c 'HashMap<String, String>' src/ir/verify/ \| awk -F: '{s+=$2} END{print s}'` → 42 |
| `.name()` render-shims | plan-writing 30; **kickoff 45**; after B **90** (Correction 2 — the retype moves each diagnostic's render to its `format!` site, and the rest are name-keyed table lookups) | `rg -c '\.name\(\)' src/ir/verify/*.rs \| awk -F: '{s+=$2} END{print s}'` |
| hand-rolled type-grammar sites (the census that matters) | kickoff 12 → after B **1** (the LINK name-domain `RES ` strip) | `rg -n 'strip_prefix\("(List OF \|Set OF \|Map OF \|RES \|Result OF \|MapEntry OF )' src/ir/verify/` |
| `HashMap<String, String>` after B | 3, all NAME→NAME (not type environments) | `rg -n 'HashMap<String, String>' src/ir/verify/` |
| verify module size | 13,481 | `find src/ir/verify -name '*.rs' \| xargs wc -l` → 13481 total (measured at kickoff) |
| distinct `TYPE_*` rules guarded by the diagnostic corpus | 124 | plan-102-F census |

### Verified properties

- **Verify never re-parses** (plan-102-F measurement: 0 runtime
  `ParameterType::parse`) — so this letter is a pure store/compare retype with
  no parse semantics to preserve. VERIFIED (recorded in plan-102-B Corrections).
- **The diagnostic corpus covers all 124 rules** (plan-102-F census:
  syntaxcheck↔verify overlap 124/124, every rule golden-guarded). VERIFIED.

## 3. Design Overview

Inside-out again: stores → `infer_type` → rule sites, one letter, two gates
(byte-identity for accepted programs, diagnostic goldens for rejected ones).
The compatibility algebra (`compat.rs`) is the risk concentration — its
string-equality edge cases (STATE-agnostic resource comparison, union
widening, `Unknown` skips) must map to structural forms that accept/reject the
exact same corpus; convert it last, behind the rest of the env.

### Rejected alternatives

- **Merge verify's engine with lowering's now-typed engine.** Rejected here:
  the soundness rule REQUIRES them independent ("verify accepts exactly what
  lowering emits" is only a check if they don't share the derivation); E
  consolidates shared *algebra*, not the walks.

## Compatibility / Format Impact

None. Diagnostics byte-identical (goldens prove it).

## Phases

Phases 1 and 2 landed together — see Correction 1.

### Phase 1 — env stores + infer_type typed

- [x] `TypeEnv` stores (`locals`/`globals`/`field_types`/`record_field_lists`/
      `FnSig`) → `ParameterType`; `infer_type -> Option<ParameterType>`;
      the 44 callers converted; `resource_base_type`/`parse_map` replaced
      structurally.
- [x] Tests: verify unit suite (`verify/tests.rs` fixtures already construct
      `ParameterType` post-plan-102) → 396 passed, 0 failed.

Detail:

- [x] All **34** `locals: &HashMap<String, …>` signatures across the nine
      modules, plus `TypeEnv`'s `globals`, `field_types` (the inner map),
      `record_field_lists` (the tuple's second slot), `FnSig::params`/`returns`
      and `current_return` → `ParameterType`. The three surviving
      `HashMap<String, String>`s are NAME→NAME maps, not type environments, and
      are correct as they stand: `resource_closers` (resource name → close op),
      `link.rs`'s `resource_state` (resource name → STATE name, over raw LINK
      AST strings), `types.rs`'s `included_members` (member name → union name).
- [x] `infer_type`/`infer_type_depth` → `Option<ParameterType>`, reading
      `annotated_parameter_type()` instead of the rendering `annotated_type()`.
- [x] `usable_type` takes and returns a `ParameterType`. Both of its rejections
      are preserved exactly: the `Unknown` **sentinel** — and the variant test is
      complete for every parsed input because `parse("Unknown")` returns the
      variant, not a nominal (`src/types.rs:274`) — and an **empty** spelling,
      which is what a malformed/hostile decoded node yields (`parse("")` →
      `Named("")`); that one stays a name test, because it is precisely a check
      on the spelling.
- [x] `resource_base_type` is structural (`Res` unwrap + a STATE strip off the
      nominal spelling); `strip_res` extracted beside it. A name-domain twin
      `resource_base_type_name` serves the `LINK` callers, whose types are raw
      un-elaborated AST strings.
- [x] `field_type`, `derived_binary_type`, `derived_unary_type`,
      `builtin_type_fields`, `field_type_map` typed.

### Phase 2 — compat algebra + remaining rule sites structural

- [x] Convert `compat.rs` (expression/binding/argument compatibility) and the
      remaining rule modules to structural comparisons; render `name()` only in
      `format!` message sites.
- [x] Tests: the full `*-invalid` corpus; the package-path decode-hardening
      vectors (crafted-`.mfp` suites).

Detail:

- [x] `compatible` matches variants (`ListOf`/`ResultOf`/`MapOf` recursion,
      `Res` unwrap) instead of the `strip_prefix` + `parse_map` cascade. Its
      **tail** stays in the name domain deliberately: bare-vs-qualified nominal
      equality (`fs.File` ≡ `File`) and union-variant membership are lookups
      keyed by the NAME an import registers.
- [x] `expression_compatible` matches `(expected, type_)` as variant pairs;
      `check_binding_type` / `check_assignment_type` / `check_return_type` /
      `check_with_update_type` / `check_result_value_type` /
      `check_member_access_type` / `check_operator_result_type` /
      `check_call_result_type` typed, rendering only inside `format!`.
- [x] `check_builtin_call_args`'s `arg_types` is `Vec<ParameterType>`, resolving
      through `resolve_call_return_type_typed` and `argument_types_typed`; the
      argument list renders once, in the diagnostic.
- [x] The predicate family — `is_comparable_seen`, `is_defaultable`,
      `contains_resource_or_thread`, `check_map_key_comparable`,
      `check_collection_res_axis`/`collection_axis_element`,
      `check_literal_range`, `read_only_record_type` — is structural. The
      `Set OF T` and `Map OF K TO V` arms of `check_map_key_comparable` were
      byte-identical; they collapse into one
      `check_collection_element_comparable`.
- [x] Deleted with their last callers: `ir::verify::parse_map`,
      `IrValue::annotated_type` (the rendering twin of
      `annotated_parameter_type`), and `numeric::money_result_type` (the
      name-keyed adapter Phase 1 of letter A added — `ir::verify` was its only
      caller). `cargo build --bin mfb` → **0 warnings**.
- [x] `is_thread_type` extracted, keeping the pre-plan-106 reach for decoded IR
      and tightening its over-match — see Correction 3.

Census after B — `rg -n 'strip_prefix("(List OF |Set OF |Map OF |RES |Result OF
|MapEntry OF )' src/ir/verify/` → **1** code site, `resource_base_type_name`'s
`RES ` strip (the LINK name-domain twin); the other three hits are prose in doc
comments recording what was replaced. `rg -n 'HashMap<String, String>'
src/ir/verify/` → 3, all NAME→NAME (enumerated above).

**Five production `ParameterType::parse` sites survive in `ir::verify`**, each
recovering a type from a spelling that has no typed value to recover *from* —
enumerated here for letter E's census rather than left for it to rediscover:

| Site | What it re-parses |
|---|---|
| `calls.rs:301`, `ops.rs:240` | the `STATE T` clause, which rides INSIDE a resource's nominal spelling (`parse` has no `STATE` arm), so the extracted `&str` must be re-classified before `is_defaultable` can judge it — this is what caught a real bug in this letter, see Correction 4 |
| `compat.rs:625`, `compat.rs:739` | a constructor's / union variant's declared type NAME, read out of the name-keyed declaration tables |
| `mod.rs:1202` | `resource_base_type`'s STATE-stripped base, same reason as the first row |

Closing these means giving the `STATE` clause a representation of its own, which
is a `ParameterType` vocabulary change and belongs to letter E's consolidation,
not to a retype letter.

Acceptance: suite green; `artifact-gate all` no NEW diff; diagnostic corpus
byte-identical. **MET**: `cargo test --bin mfb` → 3647 passed, 0 failed;
`scripts/artifact-gate.sh target/release/mfb all` →
`1255 tests, 1402 build(s), 1730 golden(s) checked, 0 diff(s)`;
`cargo test --no-fail-fast` exit 0 (62 suites, 0 FAILED); `test-accept` 1271
ran / 0 mismatches.
Commit: `7dd1301db` (both phases — see Correction 1)

## Validation Plan

- Tests: verify units; full diagnostic corpus; crafted-`.mfp` hardening suites.
- Coverage check: 124/124 rules golden-guarded (measured).
- Runtime proof: `artifact-gate all`; `test-accept` no NEW mismatch.
- Doc sync: none (E owns docs).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **`FieldTypes`-style keys:** keep `(String, String)` name keys (names are
  names, not types) — only VALUES retype. Recorded so the implementer doesn't
  over-convert.

## Corrections

### 1. Phases 1 and 2 land in ONE commit

`TypeEnv`'s stores and the compatibility algebra that reads them cannot compile
apart: `compatible`/`expression_compatible` take exactly the types the stores
hold, and all nine modules share both. Splitting the commit would leave an
uncompilable tree. Both phases are independently *verified* (the verify unit
suite after Phase 1's stores, the full corpus + gate after Phase 2); only the
commit boundary is shared. Same situation as letter A's Correction 5.

### 2. `.name()` count went UP, and that is the retype working

The plan expects `rg -c '\.name\(\)' src/ir/verify/*.rs` to fall from 30 to
"message-formatting sites only". Measured after B: **90**. That is not residue —
it is the *shape* of the conversion:

- Every diagnostic that quotes a type now renders at its `format!` site, where
  before the message interpolated an already-`String` variable. One `String`
  binding became one `.name()` call per message; the plan's own goal
  ("every message that quotes a type renders `name()` at the `format!` site")
  is what produced them.
- The rest are **name-keyed table lookups**, not type operations:
  `union_variants` / `collect_union_variants` / `records` / `unions` / `enums` /
  `field_types` / `record_field_lists` / `close_op_for` /
  `is_resource_or_resource_union` / `provably_data_type` are all keyed by the
  type NAME a declaration registers, and `state_type_name` /
  `thread_resource` read a clause that rides INSIDE a nominal's spelling
  (`parse` has no `STATE` arm).

The meaningful measure is the one letter E certifies — *hand-rolled type-grammar
parsing* and *string type environments* — and both are at their floor (see the
census in Phase 2). The plan's "30 → message sites" acceptance is replaced by
that census; a raw `.name()` count cannot distinguish a render-out from a
re-parse, which is exactly the confusion
`byte-identity-cannot-see-backward-seams` warns about.

### 3. Two unit fixtures used type strings the canonical grammar rejects

Converting `contains_resource_or_thread` / the `.result` rule from
`starts_with("Thread")` to a `ThreadHandle` variant match reddened
`rejects_thread_result_member` and `rejects_map_key_thread_ownership`. Neither
is a codegen regression; both fixtures spell types that are **not** what they
appear to be:

```
# probe, run in verify/tests.rs
ParameterType::parse("Map OF Thread OF Integer TO Integer")
  => Named("Map OF Thread OF Integer TO Integer")     # NOT a MapOf
ParameterType::parse("Thread OF Integer")
  => Named("Thread OF Integer")                       # NOT a ThreadHandle
```

`split_top_level_to` is depth-aware and correctly assigns the single ` TO ` to
the nested `Thread OF` construct, leaving the map with no value type; a handle
needs its own ` TO Out`. The deleted `parse_map` was a naive
`find(" TO ")` that split on the FIRST separator, so it "found" a map where the
canonical grammar sees a malformed nominal — the duplicate-parser divergence
`one-type-grammar-parse-is-canonical` warns about, in the direction where the
copy held the *bug*.

Both fixtures are corrected to the well-formed spellings
(`Map OF Thread OF Integer TO Integer TO Integer`,
`Thread OF Integer TO Integer`), which express what they were written to test
and now exercise the structural path. The accept/reject set is unchanged.

**And the reach is preserved.** Decoded package IR is attacker-controlled
(PKG-02) and need not be well formed, so `is_thread_type` keeps a NAME arm
beside the structural one — a crafted `.mfp` carrying the truncated spelling is
still caught, exactly as `starts_with("Thread")` caught it. That arm is pinned
by a new test, `truncated_thread_spelling_still_counts_as_a_thread`, rather than
left as untested defensive code.

### 4. `named()` on a structured spelling — the pathJoin bug class, caught again

`accepts_state_list_of_nondefaultable` reddened mid-conversion. The cause was
mine, and it is exactly the class letter A's Correction 6 found in the
`fs::pathJoin` descriptor: I had written
`is_defaultable(&ParameterType::named(state_type), …)` where `state_type` is a
spelling extracted from inside a nominal — for `fs.File STATE List OF Choice`
that spelling is `List OF Choice`. `named` wraps it as an opaque nominal, so the
`ListOf` arm of `is_defaultable` missed and a valid empty-list initial state was
reported `TYPE_STATE_INVALID`.

Fixed by using `ParameterType::parse` at all five such sites (enumerated above):
**a NAME goes back to a type through the canonical grammar, never through
`named`** — `named` is for a bare nominal only. `parse` of a bare nominal yields
the same `Named`, so it is correct in every case and cannot reintroduce the bug.

Worth recording as the letter's main lesson: this class is invisible while
everything speaks strings (a render→re-parse silently normalizes it) and becomes
a live defect the moment a consumer reads the variant. The registry half is now
guarded permanently by
`registry::tests::descriptor_named_types_are_bare_nominals` (letter A); the
compiler-internal half has no such guard, and adding one is a candidate for
letter E.

- [x] While writing that test it caught a **latent over-match in the original
      predicate**: `starts_with("Thread")` also matched an ordinary user record
      named `Threadbare`, which would be mis-reported as owning a thread handle.
      The name arm is tightened to a word boundary (`Thread`/`ThreadWorker`
      exactly, or followed by `" OF "`). Nothing relies on the over-match —
      `artifact-gate all` is 0 diffs and the full `*-invalid` corpus is
      unchanged.

## Summary

Closes plan-102-B's recorded deferral. Risk lives in the compatibility
algebra's string edge cases; two independent gates (byte-identity + the full
diagnostic corpus) hold it. After B, every engine below the front end speaks
`ParameterType`.
