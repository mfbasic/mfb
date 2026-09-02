# plan-120-D: json::stringify pretty-printing overload (M3)

Last updated: 2026-09-01
Effort: medium (1h–2h)
Depends on: plan-120-C (the compact renderer it wraps must be in its final byte shape first)

Add an indented-output form of `json::stringify`, closing review gap M3
(Node: `JSON.stringify(v, null, space)`; MFB: compact only). Output format
matches Node's exactly for the same tree — verified against the node twin —
so the two are byte-comparable after plan-120-C.

References:

- Node's layout (review probe N06): indent per depth, `": "` after keys,
  items one per line, closing bracket at parent depth, empty `[]`/`{}`
  stay inline, and `space` is clamped to 10 (number form) or the first 10
  chars (string form).
- `src/codegen/builtins/json/func_stringify.rs` — the registry entry gains
  an overload `Implementation` (arity-selected, same `Body::mfb` carrier
  pattern; overload precedent: `process::spawn`'s two implementations).
- Registry overload lore: `.ai/resources-packages.md` +
  "Overload-split beats a custom resolver".

## Prerequisites

Family gate in plan-120-A.

| Must be true | Command | Status |
|---|---|---|
| plan-120-C landed | `ls planning/plan-120-C* → planning/completed/` | NOT MET |

## 1. Goal

- `json::stringify(value, 2)` produces byte-identical output to Node's
  `JSON.stringify(v, null, 2)` for the same tree (order per plan-120-C), and
  `json::stringify(value, "\t")` matches `JSON.stringify(v, null, "\t")`.
  The 1-arg form is untouched.

### Non-goals (explicit constraints)

- The 1-arg compact form's bytes do not change.
- No replacer semantics smuggled in (M2 is deferred by decision).
- Node's clamp rules are copied as-is (Integer clamped to 0..=10; String
  truncated to 10 graphemes; 0/empty ⇒ compact) — no invented extensions.

## 2. Current State

- `func_stringify.rs` has one `Implementation` (1 param) with a `Body::mfb`
  MFBASIC body serializing recursively via string append.
- Registry overloads select by arity at codegen (spawn precedent,
  `func_spawn.rs:122-170`).
- UNMEASURED: whether stringify's body is one FUNC or delegates to per-form
  helpers — read at execution; the pretty form wants a `depth`-carrying
  variant of the same walk.

## 3. Design Overview

Two overloads added to the registry entry, both routing to one new MFBASIC
worker `__json_stringifyIndent(value, indent AS String, depth AS Integer)`:

- `stringify(value, indent AS Integer)` → worker with `indent =` that many
  spaces (clamped 0..=10; 0 ⇒ call the compact body).
- `stringify(value, indent AS String)` → worker with the first 10 graphemes
  (empty ⇒ compact).

Worker layout mirrors Node: `{\n<pad>"k": v,\n…<parentpad>}`, arrays
likewise; scalars via the existing escape/number helpers (so C's byte shape
is inherited); empty containers inline. The compact body is not modified.

Risk: low; the care point is overload resolution across the union-or-variant
first parameter (the existing single implementation already accepts `Json`
or any variant — the overloads must keep that acceptance; verify against the
registry matcher rules, "Registry strict matcher" lore).

Byte-identity: the 1-arg path is expected byte-identical (a gate!); new
overload output is new surface (behavior tests, plus a literal diff against
captured Node output for a fixture tree).

## Phases

### Phase 1 — overloads + worker

- [ ] `func_stringify.rs`: add the two `Implementation`s + the
      `__json_stringifyIndent` helper body; clamps per §1.
- [ ] Acceptance cases: a nested fixture tree rendered at 2-space, tab, 0,
      11 (→ clamped 10), empty-string (compact), empty containers inline —
      expected strings taken verbatim from Node (`JSON.stringify(v,null,x)`)
      and recorded in the test.
- [ ] DESC/EX update (`mfb man json stringify` shows both forms); man render
      + example gates.
- [ ] Prove the 1-arg path unchanged: `scripts/artifact-gate.sh all` — 0
      diffs expected for every fixture that only uses 1-arg stringify.

Acceptance: byte-equality with the recorded Node outputs for all clamp
cases; artifact-gate 0 diffs outside any new fixtures; full
`cargo test --no-fail-fast` + `scripts/test-accept.sh` green.
Commit: —

## Validation Plan

- Tests: the Node-verbatim acceptance cases; a round-trip case
  (`parse(stringify(v, 2))` = same tree).
- Doc sync: stringify DESC/EX.
- Acceptance: family standard.

## Open Decisions

- None — format is defined as "Node's, exactly", removing all layout
  bikeshed.

## Corrections

*(fill during execution)*

## Summary

Additive surface with Node's own output as the executable spec; the compact
path is pinned byte-identical so the letter cannot regress existing output.
