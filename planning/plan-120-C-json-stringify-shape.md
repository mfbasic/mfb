# plan-120-C: json::stringify byte-shape — drop `\/`, emit `-0` as `0`, specify object order

Last updated: 2026-09-01
Effort: medium (1h–2h)
Depends on: plan-120-B (family order only)

Three byte-format divergences from Node (review I6/I8/I9), fixed toward
Node parity in one golden-churn event:

- **I6**: every `/` is emitted as `\/` (`helper_escape_string.rs:18-19`);
  Node never escapes it. Fix: emit `/` raw.
- **I8**: `-0.0` emits `-0` (integer-form path of
  `helper_stringify_number.rs`); Node emits `0`. Fix: emit `0` — Node-parity
  chosen over sign preservation (identical information loss to Node's own
  round trip; documented).
- **I9**: object member order is documented as unspecified
  (`func_stringify.rs:24-26`) though the implementation is insertion-ordered
  (`src/docs/man/types/map.md:40` "insertion-ordered lookup table";
  review P18 observed document order preserved). Fix: **specify and pin**
  document/insertion order at the json level; document the permanent
  divergence from JS's integer-keys-first quirk (matching that would be
  wrong for a language without JS object semantics).

References:

- Review evidence: P17 (`"a\/b"` vs Node `"a/b"`), S06/P15 (`-0` vs `0`),
  P18 (`{"b":1,"a":2,"10":3,"2":4}` kept vs Node's `{"2":4,"10":3,...}`).
- `tests/acceptance/src/json.mfb` — pins `\/` today (e.g. the `"a\/b"`
  expectations) and possibly `-0`; census at execution.
- Map order contract: `src/docs/man/types/map.md:64-70`,
  `func_keys.rs:20-28` ("treat insertion order as the current
  implementation's behavior").

## Prerequisites

Family gate in plan-120-A.

| Must be true | Command | Status |
|---|---|---|
| plan-120-B landed | `ls planning/plan-120-B* → planning/completed/` | NOT MET |

## 1. Goal

- `json::stringify(json::parse("{\"b\":1,\"a\":\"x/y\"}"))` returns exactly
  `{"b":1,"a":"x/y"}` (document order, no `\/`), and stringifying a
  `-0.0` `JsonNum` returns `0` — all three now byte-identical to Node for
  these inputs.

### Non-goals (explicit constraints)

- Parsing is untouched (`\/` INPUT stays accepted — it is valid JSON).
- No sorting of keys, no JS integer-first emulation: the order contract is
  "the `JsonObj` map's insertion order", which for a parsed document is
  document order.
- The Map type's own contract stays "implementation-defined but stable" —
  json's guarantee is a json-level promise pinned by json tests, not a
  collections contract change (if the map implementation ever reorders,
  the json pin breaks loudly and forces that conversation).

## 2. Current State

- Escape: `helper_escape_string.rs:18-19` explicit `/` arm.
- `-0`: integer path emits `toString(value, 0)` = `-0` and round-trips via
  `toFloat("-0")` (review S06 `OK -0`).
- Order: emitted from `FOR EACH` over the `JsonObj` map (stringify body);
  observed insertion order; docs disclaim it.
- Acceptance pins of `\/`: UNMEASURED count —
  `grep -c '\\\\/' tests/acceptance/src/json.mfb` at execution; each flipped
  pin is this plan working (documented divergence removed), not a weakened
  test.

## 3. Design Overview

Three edits, one behavioral commit:

1. Delete the `/` arm from `__json_escapeString` (the raw grapheme falls
   through to the pass-through arm).
2. `__json_stringifyNumber`: after the integer-form text is produced, map
   exactly `-0` → `0` (the parse-back check compares against `value`;
   `toFloat("0") = -0.0` is true under IEEE `=`, so the round-trip
   verification still passes — verify this in a unit case, it is the one
   subtle spot).
3. Docs: `func_stringify.rs` DESC — replace the two "not byte-identical to
   other writers" / "do not rely on order" paragraphs with the new
   contracts; note the JS integer-first divergence explicitly.

Byte-identity NOT a gate: json-fixture goldens churn by design; regenerate
and prove the delta list = json-importing fixtures only.

Rejected: keeping `-0` (more faithful, but the user's driver is Node
interop and Node's own stringify already loses the sign — parity wins);
emitting integer-first order (JS-object artifact).

## Phases

### Phase 1 — the three edits

- [ ] `helper_escape_string.rs`: drop the `/` arm.
- [ ] `helper_stringify_number.rs`: `-0` → `0` with the IEEE-equality
      round-trip note; unit-style acceptance case proving `stringify` of a
      parsed `-0` is `0` and re-parses to `-0.0`.
- [ ] `func_stringify.rs` DESC: new escape/order/`-0` contract text.
- [ ] `tests/acceptance/src/json.mfb`: flip the `\/` and `-0` pins; ADD the
      order pin (parse `{"b":1,"a":2,"10":3,"2":4}` → stringify equals the
      input byte-for-byte).
- [ ] Regenerate churned goldens; man render + example gates for the json
      pages.

Acceptance: goal example exact bytes; the order pin green; full
`cargo test --no-fail-fast` + `scripts/test-accept.sh` full count +
`scripts/artifact-gate.sh all` (regenerated, delta confined); fmt + check
`--all-targets`.
Commit: —

## Validation Plan

- Tests: the flipped/added acceptance cases; a cross-check case feeding the
  new output to `json::parse` (round trip stays identity).
- Doc sync: stringify DESC; the review-era claim in `mod.rs` MODULE_DESC
  ("emits object pairs in the map's iteration order… not guaranteed") gets
  the new wording too.
- Acceptance: family standard.

## Open Decisions

- None — the `-0`→`0` choice is settled above (Node parity) unless review
  during execution surfaces a consumer relying on `-0`; if one exists,
  reopen with that evidence.

## Corrections

*(fill during execution)*

## Summary

Small, mechanical, and the one place this family deliberately trades MFB
faithfulness (`-0`) for Node parity; the order guarantee is promoted from
observed behavior to a pinned contract without touching the Map type.
