# bug-444: binding a fallible union-returning call through TRAP fails codegen — "native code cannot materialize default value for type '<Union>'"

Last updated: 2026-08-16
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (codegen feature gap — rejects valid source)

Status: Open
Regression Test: tests/rt-behavior/rt_union_trap_bind_default.rs (to be added)

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

1. **Failing test + audit (no behavior change).** Add
   `tests/rt-behavior/rt_union_trap_bind_default.rs` that builds+runs the
   minimal repro above (and a user-`UNION` variant). It must fail today with the
   materialize error. NB: this must be a full-`mfb build` fixture — the error is
   raised in native lowering, so a `tests/syntax/*` (`-ast -ir`) fixture would
   pass spuriously and not guard the path (per project testing-gates notes).
   Commit:
2. **Add a data-union arm to `lower_default_value`.** Before the final record
   arm, match a type present in `self.type_model.union_variant_fields`: pick a
   defaultable variant (the first variant is the natural choice; confirm its
   payload record is itself defaultable via the existing record path), build
   that variant's default record, and emit the `{ tag, variant-record-ptr }`
   union value with the chosen variant's tag. Mirror the resource-union tag
   discipline in `emit_resource_record_ptr`/union construction so the layout is
   identical to a normal union value. Commit:
3. **Verify byte-neutrality + full suite.** Confirm no golden/`.ncode` drift for
   programs that do not bind a fallible union through TRAP (the new arm is only
   reached for the data-union default). Run the full `cargo test` and the
   acceptance harness. Commit:

## Notes

- Workaround in the field (no compiler change): route the fallible union call
  through a helper that returns a record/scalar and TRAP the call to *that*
  helper. `examples/ai_chat`'s `parseStreamLine` (returns a `Parsed` record) is
  a worked example.
