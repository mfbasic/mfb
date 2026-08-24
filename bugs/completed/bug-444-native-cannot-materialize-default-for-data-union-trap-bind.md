# bug-444: binding a fallible union-returning call through TRAP fails codegen — "native code cannot materialize default value for type '<Union>'"

Last updated: 2026-08-16
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (codegen feature gap — rejects valid source)

Status: FIXED
Regression Test: tests/rt_union_trap_bind_default.rs (cargo) +
tests/rt-behavior/trap/inline-trap-union-bind-rt (acceptance golden fixture)

Binding the result of a **fallible call whose return type is a data UNION** to a
`LET`/`MUT` with a local `TRAP` fails native codegen with
`native code cannot materialize default value for type '<Union>'`. It happens
for both the diverging-handler and the `RECOVER`-value forms, because the
`... TRAP ... END TRAP` desugar always emits a `bind $trap_valN : <Union> =
<default>` temp, and native lowering has no default-value path for a data union.
The single correct behavior a fix produces: such a bind **compiles**, and the
default materialized for the never-observed trap temp is a well-formed default
union value (the first/defaultable variant carrying its own default payload);
on the taken error path the `RECOVER` value (or divergence) supersedes it, so no
program ever observes the synthesized default.

The most common trigger in real code is `json::parse`, whose return type is the
`json::Json` union — so *any* program that wants to catch a JSON parse failure at
the binding site is blocked and must route the fallibility through a helper that
returns a record/scalar instead (the workaround used by `examples/ai_chat`).

References:

- `mfb spec` §15.6 / `mfb spec memory heap-values` — union value layout is
  `{ tag@0, variant-record-ptr@8 }`.
  **Correction (found while fixing):** that layout is the RESOURCE-union form.
  A *data* union is the flat `{ tag@0, size@8, variant-record-block inlined@16 }`
  layout (plan-02 §4.3; `emit_wrap_record_in_union` in
  `src/codegen/collection/layout/builder_collection_layout.rs`), and the fix
  emits that form.
- Sibling path that already works: resource unions and concrete resources get a
  materialized default (a closed record) in the same function — see Root Cause.
- Found while building `examples/ai_chat` (parsing `claude --output-format
  stream-json` output with `json::parse`); worked around there by isolating
  `json::parse` inside `parseStreamLine`, which returns a plain record, and
  TRAPping the call to *that* (a record default IS materializable).

## Failing Reproduction

Minimal program:

```basic
IMPORT json
FUNC main() AS Integer
  LET d AS json::Json = json::parse("{}") TRAP(e)
    RECOVER JsonNull[NOTHING]
  END TRAP
  RETURN 0
END FUNC
```

```
mfb build <project>
```

- Observed: `error: native code cannot materialize default value for type 'Json' while lowering bind $trap_val1 AS Json` (build fails, no executable).
- Expected: builds and runs; the program returns 0 (parse succeeds here, and even on a parse failure the `RECOVER` value is used, not the synthesized default).

The diverging-handler form fails identically:

```basic
LET d AS json::Json = json::parse(line) TRAP(e)
  RETURN Parsed["", ""]   ' handler diverges -> still needs a default trap temp
END TRAP
```

Contrast cases that compile today (these bound the bug and become regression
guards):

- **No TRAP** (auto-propagate) — `LET d AS json::Json = json::parse(x)` compiles;
  no trap temp is created, so no default is needed. (This is how the json
  package's own callers and the `ai_chat` workaround avoid it.)
- **Fallible RECORD-returning call through TRAP** — compiles; a record default is
  materialized field-by-field. (This is the `ai_chat` workaround: TRAP the call
  to a `FUNC ... AS Parsed`.)
- **Fallible SCALAR/String-returning call through TRAP** — compiles.
- **Fallible RESOURCE / resource-UNION-returning call through TRAP** — compiles;
  a *closed* resource record is materialized (see Root Cause), so resource unions
  are unaffected. Only **data** unions hit the gap.

| Environment | arch | Result |
| --- | --- | --- |
| macOS | aarch64 (debug `mfb`) | fails ✗ |

(Arch-neutral: the failure is in shared native-lowering code, not a per-arch
backend, so every native target is expected to reproduce.)

## Root Cause

`lower_default_value` in
`src/target/shared/code/builder_value_semantics.rs` materializes the default
value the `TRAP` desugar's `bind $trap_valN : T = <default>` temp requires. It
has arms for:

- scalars/String/collections,
- resources and resource unions — via `is_resource_type` /
  `resource_names.contains(base_resource_name(...))` → `emit_closed_resource_record`
  (`builder_value_semantics.rs:211`), and
- **records** — the final `_ =>` arm looks up `self.type_model.record_fields.get(type_)`
  and recurses over the fields (`builder_value_semantics.rs:243`).

A **data union** (`json::Json`, or any user `UNION` of non-resource variants) is
neither a scalar, nor a resource, nor a record, so it falls into that final arm;
`record_fields.get("Json")` is `None`, and the arm returns the hard error
`native code cannot materialize default value for type '{type_}'`
(`builder_value_semantics.rs:246`). There is simply no code path that builds a
default union value.

Why the contrast cases are immune: auto-propagate creates no trap temp (nothing
to default); records recurse into `record_fields`; resources/resource-unions take
the closed-record arm above. Only the data-union case reaches the unhandled
`_ =>`.

## Non-goals

- **Do not** change the union value layout (`{ tag@0, variant-record-ptr@8 }`),
  the fallible-call ABI, or any golden/`.ncode` output for programs that don't
  bind a fallible union through TRAP. The fix adds a new default-materialization
  path; it must be byte-neutral for existing programs.
- **Do not** change source semantics: the synthesized default is for a
  never-observed temp on the untaken path. `RECOVER`/divergence semantics,
  `TRAP` routing, and union `MATCH` exhaustiveness are unchanged.
- **Do not** "fix" this by making `json::parse` (or the json package) avoid
  unions, or by documenting the limitation and rewriting the repro to dodge the
  broken path — the bind of a fallible union through TRAP is valid source and
  must compile.
- Resource unions already work (closed-record path) and must stay unchanged.

## Blast Radius

- **json package callers** — every user program that binds `json::parse` (the
  only common fallible union-returning builtin) through a `TRAP` is blocked
  today. Fixed by this bug.
- **User-defined data unions** — a `FUNC f(...) AS SomeUnion` that is fallible,
  bound through TRAP, has the same gap. Fixed by this bug (the default path is
  generic over data unions, keyed on `type_model.union_variant_fields`).
- **Resource unions** — unaffected; take the existing closed-record arm. Verify
  the new arm is ordered *after* the resource checks so it does not intercept
  them.
- **Auto-propagating / non-TRAP union binds** — unaffected (no trap temp).

## Fix (phased, test-first)

1. [x] **Failing test + audit (no behavior change).** Add
   `tests/rt_union_trap_bind_default.rs` (cargo integration test; the plan's
   suggested `tests/rt-behavior/*.rs` path mixed the two conventions) that
   builds+runs the minimal repro above (and a user-`UNION` variant). It must
   fail today with the materialize error. NB: this must be a full-`mfb build`
   test — the error is raised in native lowering, so a `tests/syntax/*`
   (`-ast -ir`) fixture would pass spuriously and not guard the path (per
   project testing-gates notes).
   Commit: b5cd0650e (RED-verified: all 3 tests failed with the exact
   materialize error for 'Json'/'Shape')
2. [x] **Add a data-union arm to `lower_default_value`.** Before the final
   record arm, match `union_is_data` (ordered after the resource checks): pick
   the first canonically-ordered variant whose payload is statically
   defaultable (`default_value_materializable` mirrors the emission arms, with
   a defaulting-unions cycle guard threaded through
   `lower_default_value_inner`), build that variant's default record via the
   existing record path, and wrap it with `emit_wrap_record_in_union` — the
   canonical flat data-union layout `{tag@0, size@8, record@16}` (NOT the
   doc's `{tag, ptr}` sketch, which is the resource-union form; see the
   References correction). Also adds the acceptance golden fixture
   `tests/rt-behavior/trap/inline-trap-union-bind-rt`.
   Commit: 83ad3f0f5 (rt_union_trap_bind_default 3/3 green; fixture passes
   test-accept)
3. [x] **Verify byte-neutrality + full suite.** Confirm no golden/`.ncode`
   drift for programs that do not bind a fallible union through TRAP (the new
   arm is only reached for the data-union default). Run the full `cargo test`
   and the acceptance harness. Commit: (see STATUS below)

## Notes

- Workaround in the field (no compiler change): route the fallible union call
  through a helper that returns a record/scalar and TRAP the call to *that*
  helper. `examples/ai_chat`'s `parseStreamLine` (returns a `Parsed` record) is
  a worked example. (Obsolete once this fix lands — the direct bind compiles.)

## STATUS: FIXED (83ad3f0f5)

Fixed on `worktree-B-444` and merged to main. Verification record:

- Repro: `mfb build` of the doc's minimal program (and the diverging form, and
  a user-`UNION` equivalent) builds and runs, exit 0; only environment in the
  matrix was macOS aarch64 and it passes there
  (`target/release/mfb build /tmp/bug444-wt` + run).
- Regression tests: `cargo test --test rt_union_trap_bind_default` — RED 0/3
  before the arm (exact materialize error for 'Json'/'Shape'), GREEN 3/3 after.
- Byte-neutrality: `artifact-gate [all]: 1249 tests, 1396 build(s), 1718
  golden(s) checked, 0 diff(s)` (run inside the full `cargo test` via
  `tests/golden.rs::artifact_gate_all`, release mfb). No pre-existing golden
  shifted; the only new goldens are the new fixture's own.
- Full suite: `cargo test --no-fail-fast` exit 0 — 62/62 targets
  `test result: ok`, 0 FAILED.
- Acceptance: full `test-accept.sh` run had 1 phantom mismatch
  (`rt-behavior/collections/map-removekey-inplace-rt`, a `tests/tests/...`
  doubled-path actual + missing ast/ir) caused by a CONCURRENT foreign
  session's test-accept writing the same `/tmp/accept-out` actual dir (the
  documented concurrent-clobber failure mode). Proven phantom by hand: the
  fixture's `-ast -ir` dumps diff clean against its goldens, and its built
  executable's run output is byte-identical to the golden `build.log` run
  section on this compiler. Unrelated to this change.

Deviations from the doc's fix sketch:

- The doc's union layout `{tag@0, variant-record-ptr@8}` is the RESOURCE-union
  form; the fix emits the actual data-union layout
  `{tag@0, size@8, variant-record-block@16}` via `emit_wrap_record_in_union`
  (References correction above).
- The regression test lives at `tests/rt_union_trap_bind_default.rs` (cargo
  integration test), not the doc's `tests/rt-behavior/*.rs` path, which mixed
  the fixture-tree and cargo-test conventions; a golden fixture
  `tests/rt-behavior/trap/inline-trap-union-bind-rt` covers the fixture tree.
- Variant choice is the first canonically-ordered variant whose payload is
  *statically* defaultable (`default_value_materializable`, a mirror of the
  emission arms with a defaulting-unions cycle guard), not blindly the first
  variant — so a self-reachable union picks a variant that terminates.
