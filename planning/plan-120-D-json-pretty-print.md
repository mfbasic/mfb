# plan-120-D: json::stringify pretty-printing overload (M3)

Last updated: 2026-09-02
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

  **Re-captured verbatim from Node v24.12.0 during plan-120-A's execution**
  (`/Users/justinzaun/local/bin/node` — the oracle this letter needs is present).
  Every clamp rule above is confirmed. For
  `{b:1, a:"x/y", n:[1,2.5,true,null], o:{k:{}}, e:[]}`:

  `JSON.stringify(v, null, 2)`:

  ```
  {
    "b": 1,
    "a": "x/y",
    "n": [
      1,
      2.5,
      true,
      null
    ],
    "o": {
      "k": {}
    },
    "e": []
  }
  ```

  - `space=0` and `space=""` both produce the compact form byte-for-byte
    (`{"b":1,"a":"x/y",...}`), so both route to the 1-arg body.
  - `space=11` indents by exactly 10 spaces per level — the numeric clamp.
  - `space="\t"` indents one tab per level.
  - `space="abcdefghijklmno"` indents with `abcdefghij` — the **first 10
    characters**, repeated once per depth level (so depth 2 is
    `abcdefghijabcdefghij`). Note this is a UTF-16 code-unit / character
    truncation in Node, not a grapheme one; for the ASCII indents anyone
    actually uses the distinction cannot be observed, and the plan should say
    "characters" rather than "graphemes".
  - Empty containers stay inline even in pretty mode: `"e": []` and `"o": {}`
    on one line, never expanded to `[\n  ]`.
  - Nested empties too: `"o": {\n    "k": {}\n  }` — the OUTER object expands,
    the inner empty one does not.
- `src/codegen/builtins/json/func_stringify.rs` — the registry entry gains
  an overload `Implementation` (arity-selected, same `Body::mfb` carrier
  pattern; overload precedent: `process::spawn`'s two implementations).

  **Better precedent, located during plan-120-A's execution:**
  `datetime::parse` (`src/codegen/builtins/datetime/func_parse.rs:126,147`) is
  two arity-selected `Implementation`s **both carried by `Body::mfb`** —
  `Body::mfb(BODY_2, "__datetime_parse2")` and
  `Body::mfb(BODY_3, "__datetime_parse3")`. `process::spawn`'s overloads are
  `Body::abi_function_aliased`, i.e. native lowering, so they answer a
  different question. Mirror `datetime::parse`: one `Implementation` per arity,
  each naming its own `__json_*` body function.
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
  truncated to 10 **characters** — corrected from "graphemes", see the
  Node capture in References; 0/empty ⇒ compact) — no invented extensions.

## 2. Current State

- `func_stringify.rs` has one `Implementation` (1 param) with a `Body::mfb`
  MFBASIC body serializing recursively via string append.
- Registry overloads select by arity at codegen (spawn precedent,
  `func_spawn.rs:122-170`).
- ~~UNMEASURED: whether stringify's body is one FUNC or delegates to per-form
  helpers~~ — **MEASURED (plan-120-A execution): it is ONE recursive FUNC**,
  `__json_stringify(value AS Json) AS String`
  (`func_stringify.rs:103-146`), a single `MATCH` over the six variants that
  recurses directly (`__json_stringify(item)` for array items,
  `__json_stringify(entry.value)` for object members) and delegates only the
  two leaf renderings, to `__json_stringifyNumber` and `__json_escapeString`.

  So the pretty form is a `depth`-carrying **clone** of that one `MATCH`, not a
  wrapper over per-form helpers — and it inherits plan-120-C's byte shape for
  free, because the two leaf helpers are shared and unchanged. The compact body
  is not touched, which is what makes the "1-arg path byte-identical" gate
  achievable.

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

**D-C1 — "10 graphemes" should be "10 characters".** Measured against Node
v24.12.0 (capture in References): `JSON.stringify(v, null, "abcdefghijklmno")`
indents with `abcdefghij`, i.e. Node truncates `space` by UTF-16 code units, not
by grapheme clusters. Corrected in the Non-goals above. The distinction is
unobservable for the ASCII indents anyone writes, but "copy Node's rules
exactly" is this letter's whole specification, so the wording has to match what
Node does rather than what MFB's string vocabulary would default to.

## Summary

Additive surface with Node's own output as the executable spec; the compact
path is pinned byte-identical so the letter cannot regress existing output.
