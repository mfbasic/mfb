# plan-120-C: json::stringify byte-shape — drop `\/`, emit `-0` as `0`, specify object order

Last updated: 2026-09-02
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

  **Re-captured verbatim from Node v24.12.0 during plan-120-A's execution**, so
  this letter's expected strings are oracle output rather than recollection:

  ```
  JSON.stringify("a/b")                          -> "a/b"
  JSON.stringify(-0)                             -> 0
  JSON.stringify({b:1,a:2,"10":3,"2":4})         -> {"2":4,"10":3,"b":1,"a":2}
  JSON.stringify(["\"","\\","/","\n","\t","\r","\b","\f"])
                                                 -> ["\"","\\","/","\n","\t","\r","\b","\f"]
  ```

  Two things this settles:

  - The full escape set is confirmed unchanged apart from `/`: `"`, `\`, and the
    five C0 shorthands all still escape, so deleting only the `/` arm is exactly
    right and the letter's 242-line acceptance pin changes in only that one run.
  - The **object-order divergence is real and permanent**, and larger than "JS
    puts integer keys first": Node reorders `10` before `b` *and* sorts the
    integer-like keys ascending (`"2"` then `"10"`), i.e. it is not insertion
    order at all for those keys. §1's Goal stays accurate because its example
    (`{"b":1,"a":"x/y"}`) has no integer-like keys and so IS Node-identical —
    but the order pin this letter adds is deliberately NOT Node parity, and the
    DESC must say so plainly rather than implying byte-equality in general.
- `tests/acceptance/src/json.mfb` — pins `\/` today (e.g. the `"a\/b"`
  expectations) and possibly `-0`; census at execution.
- Map order contract: `src/docs/man/types/map.md:64-70`,
  `func_keys.rs:20-28` ("treat insertion order as the current
  implementation's behavior").

  **Read during plan-120-A's execution — the exact wording matters for how
  strong a promise this letter can make.** `map.md`'s "Iteration order"
  section says order is "implementation-defined but stable for a given
  unchanged map value during one program run", and — the load-bearing clause —
  "**Creating a changed map value may choose a different order.**" A parsed
  document builds its map by successive insertions, i.e. by creating changed
  map values repeatedly, so document-order output is *not* guaranteed by the
  Map contract as written; it follows from the storage layout
  (`map.md:38`: "a header, an **insertion-ordered lookup table**, a packed data
  region, and a derived hash index"), which appends.

  This does not block the letter — it sharpens why §3's design is right. The
  json-level pin is the tripwire that turns a future layout change from a
  silent output change into a loud test failure, and it must therefore assert
  document order on a MULTI-key parsed document (the plan's
  `{"b":1,"a":2,"10":3,"2":4}` case does exactly that). What the letter must
  NOT do is restate the Map contract as ordered; C's Non-goals already forbid
  that.

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
- **The native formatter is not touched.** `float_format.rs`'s header states
  the sign-preserving behaviour as a deliberate contract — "`-0.0` renders with
  the sign (`-0.00`)" — so `toString(-0.0, toByte(0))` returning `"-0"` is
  correct and stays correct. (Node differs here at the formatter level too:
  `(-0).toFixed(0)` is `"0"`.) The `-0` → `0` mapping therefore belongs in
  `__json_stringifyNumber` alone, as §3.2 says; "fixing" the formatter instead
  would silently change `toString(Float)`'s public output for every caller.
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
  `toFloat("-0")` (review S06 `OK -0`). **The plan wondered whether `-0` was
  pinned; measured (plan-120-A execution) — it is, once:**
  `tests/acceptance/src/json.mfb:102`, `expectString(pv("-0"), "-0")`. That
  single pin flips to `expectString(pv("-0"), "0")`, and the case should gain
  the round-trip half the plan's §3.2 calls the subtle spot (re-parsing `0`
  still yields a value `= -0.0` under IEEE equality).
- Order: emitted from `FOR EACH` over the `JsonObj` map (stringify body);
  observed insertion order; docs disclaim it.
- Acceptance pins of `\/`: **MEASURED (during plan-120-A's execution) — 2**,
  via `grep -c '\\\\/' tests/acceptance/src/json.mfb`:

  | Line | Pin |
  |---|---|
  | 131 | `expectString(pv("\"a\\/b\""), "\"a\\/b\"")` — parse-then-stringify round trip |
  | 242 | `expectString(json::stringify(json::JsonStr[escaped]), "\"\\\"\\\\\\/\\n\\t\\r\\b\\f\"")` — the whole escape set in one string |

  (Line numbers are as of **after plan-120-A landed**; A added cases to the
  parse-security group and B adds cases to the get/getOr groups, both of which
  sit clear of these two, so the numbers hold through B. Re-grep rather than
  trusting them if anything else lands first.)

  Both flip: 131's expectation becomes `"\"a/b\""` (the INPUT keeps its `\/`,
  which stays valid on the parse side), and 242 loses the `\\/` run from the
  middle of the escape-set expectation. Each flipped pin is this plan working
  (documented divergence removed), not a weakened test.

  No `.mfb` fixture outside `tests/acceptance/` pins `\/`; the four
  `tests/rt-behavior/json/*/golden/*.ir` hits are the json package body
  rendered into the IR dump, which regenerate mechanically.

  Note for the man pages: `func_stringify.rs`'s second EX block prints
  `json::stringify(json::JsonStr["a/b"])` (`func_stringify.rs:86`), whose
  *observed* output under `man-run-examples.sh json --run` during letter A was
  `"a\/b"` and becomes `"a/b"` here. The EX block itself declares no expected
  output, so nothing in it needs editing — but the DESC's "**String
  escaping**" paragraph opens by saying `/` is escaped "— every forward slash
  is emitted as `\/`", and that whole clause plus its "not byte-identical to
  what most other JSON writers produce" follow-on is what this letter deletes.

## 3. Design Overview

Three edits, one behavioral commit:

1. Delete the `/` arm from `__json_escapeString` (the raw grapheme falls
   through to the pass-through arm).

   **Fallthrough confirmed by reading (plan-120-A execution).** The arms after
   the deleted one are the C0 escapes (`\n`, `\t`, `\r`, `\u{8}`, `\u{C}` —
   none of which `/` equals) and then
   `ELSEIF __json_isRawControlChar(ch)`. That predicate
   (`helper_is_raw_control_char.rs:14-20`) returns TRUE only for a single
   scalar `< 32`; `/` is U+002F = 47, so it answers FALSE and `/` reaches the
   `ELSE out = out & ch` pass-through. Deleting the arm is therefore the whole
   change — no other arm has to be touched, and no arm ordering shifts.
2. `__json_stringifyNumber`: after the integer-form text is produced, map
   exactly `-0` → `0` (the parse-back check compares against `value`;
   `toFloat("0") = -0.0` is true under IEEE `=`, so the round-trip
   verification still passes — verify this in a unit case, it is the one
   subtle spot).

   **The IEEE-equality assumption was checked ahead of time (plan-120-A
   execution) and holds**: MFBASIC `Float` `=` lowers to `abi::float_compare_d`
   (`src/target/shared/abi.rs:1314`, emitting `fcmp_d`), i.e. the hardware IEEE
   compare, under which `+0.0 == -0.0` is true. So with the mapping applied
   *before* the round-trip check, `toFloat("0")` yields `+0.0`, `value` is
   `-0.0`, and `IF toFloat(integerText) = value` is still TRUE — the integer
   branch returns `"0"` rather than falling through to the fractional search.
   The unit case the plan asks for is still worth adding: it pins the behavior,
   not the reasoning.
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
