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
| plan-106-A complete | `ir::lower::expression_type` returns `Option<ParameterType>`; A's boxes ticked | NOT MET until A lands |

## 1. Goal

- `TypeEnv`'s stores are `ParameterType`-valued; `infer_type` returns
  `Option<ParameterType>`; the rule implementations compare structurally.
- The diagnostic **text, codes, and order are byte-identical** — every message
  that quotes a type renders `name()` at the `format!` site.
- `rg -c '\.name\(\)' src/ir/verify/*.rs` drops from 30 to only
  message-formatting sites (each listed in the acceptance).
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
| `.name()` render-shims | 30 | `rg -c '\.name\(\)' src/ir/verify/*.rs \| awk -F: '{s+=$2} END{print s}'` → 30 |
| verify module size | — | `find src/ir/verify -name '*.rs' \| xargs wc -l` (record at kickoff) |
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

### Phase 1 — env stores + infer_type typed

- [ ] `TypeEnv` stores (`locals`/`globals`/`field_types`/`record_field_lists`/
      `FnSig`) → `ParameterType`; `infer_type -> Option<ParameterType>`;
      the 44 callers converted; `resource_base_type`/`parse_map` replaced
      structurally.
- [ ] Tests: verify unit suite (`verify/tests.rs` fixtures already construct
      `ParameterType` post-plan-102).

Acceptance: suite green; `artifact-gate all` no NEW diff; diagnostic corpus
byte-identical.
Commit: —

### Phase 2 — compat algebra + remaining rule sites structural

- [ ] Convert `compat.rs` (expression/binding/argument compatibility) and the
      remaining rule modules to structural comparisons; render `name()` only in
      `format!` message sites.
- [ ] Tests: the full `*-invalid` corpus; the package-path decode-hardening
      vectors (crafted-`.mfp` suites) — the sole guard the review flags at
      `Compiler Pipeline.md:45`.

Acceptance: suite green; `artifact-gate all` no NEW diff; diagnostic corpus
byte-identical; `.name()` census recorded (message-format sites only);
no-backward grep per plan-104-A's pattern.
Commit: —

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

<Filled in during execution.>

## Summary

Closes plan-102-B's recorded deferral. Risk lives in the compatibility
algebra's string edge cases; two independent gates (byte-identity + the full
diagnostic corpus) hold it. After B, every engine below the front end speaks
`ParameterType`.
