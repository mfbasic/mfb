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
| plan-120-C landed | `ls planning/plan-120-C* → planning/completed/` | **MET** — C committed as `eba44c00e` on `worktree-P-120` with every gate green (95 cargo-test binaries / 0 failures, 743/743 acceptance, artifact-gate 1830 goldens / 0 diffs). This is the dependency that actually matters for this letter: the compact renderer's byte shape is now final (no `\/`, `-0` → `0`, pinned member order), so the pretty form inherits it rather than being written against a shape that then moves. |

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

- [x] `func_stringify.rs`: add the two `Implementation`s + the
      `__json_stringifyIndent` helper body; clamps per §1.
      Three helpers rather than one: `__json_stringifyIndent` (the depth-carrying
      walk) plus `__json_indentFromCount` / `__json_indentFromText` for the two
      clamps, so each overload's body is three lines and the clamp rules are
      stated once each. Both 2-arg bodies route back into `__json_stringify`
      when the clamped indent is empty, so there is exactly one compact renderer.
      **The same-arity type dispatch needed a precedent the plan did not have —
      and then exposed a compiler bug. See Corrections D-C2 and D-C3.**
- [x] Acceptance cases: a nested fixture tree rendered at 2-space, tab, 0,
      11 (→ clamped 10), empty-string (compact), empty containers inline —
      expected strings taken verbatim from Node (`JSON.stringify(v,null,x)`)
      and recorded in the test.
      Seven `TCASE`s. Beyond the listed set they also pin: a negative count
      (compact, not an error), `stringify(v, 0)` equalling `stringify(v)`
      exactly, a scalar root having nothing to indent, the pretty output
      re-parsing to the same tree, and plan-120-C's `\/` and `-0` rules holding
      in the indented form too (they are inherited, not reimplemented).
- [x] DESC/EX update (`mfb man json stringify` shows both forms); man render
      + example gates.
      The rendered page shows all three declarations. Two new examples, each
      with its expected output; `man-run-examples.sh json --run` → **16/16**
      built and ran, and both print exactly what the page documents including
      the empty `[]` staying inline. `man-census.sh --memory-scope` → 0 hits.
- [x] Prove the 1-arg path unchanged: `scripts/artifact-gate.sh all` — 0
      diffs expected for every fixture that only uses 1-arg stringify.

      **Proven, and by a stronger argument than the gate alone.**
      `artifact-gate.sh all` → **1830 goldens checked, 0 diffs.** But the gate
      runs after regeneration, so on its own it would only show the goldens
      match what the compiler now emits. The claim that the 1-arg PATH is
      unchanged rests on the pre-regeneration `.ir` diff: the json IR gained
      exactly the five new functions (`#json_indentFromCount`,
      `#json_indentFromText`, `#json_stringifyCount`, `#json_stringifyIndent`,
      `#json_stringifyText`) and removed **nothing**. The only apparent
      removals were `$matchN` temporaries renumbering — the counter is
      module-wide, so the new helper's `MATCH` claimed `$match2` and pushed
      `get`/`getOr` to `$match3`/`$match4`; both were confirmed present and
      intact with unchanged bindings.

      Three acceptance cases pin the behavioural half directly:
      `stringify(v, 0)` and `stringify(v, "")` are asserted EQUAL to
      `stringify(v)`, not merely equal to a literal — so the compact form
      cannot drift from itself.

      **Blast-radius note for reading this gate.** Correction D-C3's fix is in
      `ir/lower.rs`'s `expected_parameter_type`, which is shared by EVERY
      builtin call, so its reach is wider than json. It can only ever ADD an
      expected type where there was none, and only for an OVERLOADED member at a
      position where all overloads agree — so the fixtures at risk are those
      calling an overloaded builtin with an argument that was previously left
      unwrapped. Any such diff is the fix working (a union argument that should
      always have been wrapped now is), not collateral. A diff on a
      single-implementation member would NOT be explainable that way and would
      mean the change reaches further than intended — check for that
      specifically rather than regenerating on sight.

Acceptance: byte-equality with the recorded Node outputs for all clamp
cases; artifact-gate 0 diffs outside any new fixtures; full
`cargo test --no-fail-fast` + `scripts/test-accept.sh` green.

**MET.** Byte-equality was established by writing the same tree and the same
six clamp cases in both languages and `diff`-ing the full transcripts — not by
eyeballing a sample:

```
$ diff /tmp/p120-D-node.txt /tmp/p120-D-mfb.txt
=== BYTE-IDENTICAL TO NODE ===
```

covering `space` = 2, 0, 11 (clamped to 10), `"\t"`, `""`, and
`"abcdefghijklmno"` (truncated to 10), plus the 1-arg compact form. Re-run
after Correction D-C3's fix and still byte-identical, so the fix did not buy
member-type dispatch at the cost of layout.

- `mfb test tests/acceptance` → **750 pass, 0 fail** (743 before).
- `cargo test --no-fail-fast` → **exit 0, 95 binaries, 0 failures**, 3729 unit
  tests (up one: the new `agreed_argument_type` regression test).
  `no_golden_pins_a_fatal_signal` is green, which is the check that would have
  caught D-C5 had the crashed golden been regenerated.
- `scripts/test-accept.sh` → 1349 ran, 9 mismatches: 8 legitimate (7 json
  `.ir` + the diagnostic `build.log` of D-C6) and 1 environmental crash left
  untouched (D-C5). Regenerated the 8; `regen-ncodesum.sh` moved only the 5
  json sums out of 141 refreshed.
- `scripts/artifact-gate.sh all` → **1830 goldens, 0 diffs**, with
  `datetime::parse`, `process::spawn`, `tls::poll` and `http`'s `Func`-typed
  `Route` field all passing — the overloaded builtins D-C3's shared-seam change
  can reach, confirming it is inert where the overloads disagree.
- `man-run-examples.sh json --run` → **16/16**; `man-census.sh --memory-scope`
  → 0 hits; `cargo check --all-targets` clean; `cargo fmt` both roots, no churn.
Commit: b08678cbf

## Validation Plan

- Tests: the Node-verbatim acceptance cases; a round-trip case
  (`parse(stringify(v, 2))` = same tree).
- Doc sync: stringify DESC/EX.
- Acceptance: family standard.

## Open Decisions

- None — format is defined as "Node's, exactly", removing all layout
  bikeshed.

## Corrections

**D-C5 — one golden diff in this letter's run was a CRASH, and was correctly NOT
regenerated.** `scripts/test-accept.sh` reported 9 mismatches, one of them
`rt-behavior/tls/tls-poll-rt/build.log`:

```
< [exit 0]
> [exit 139]
```

139 is SIGSEGV. `tls::poll` IS overloaded (`Socket` vs `List OF Socket`) and
shares one `timeoutMs` parameter across both overloads, so Correction D-C3's
`agreed_argument_type` change genuinely reaches it — this was the exact shape
the blast-radius note in Phase 1 said to scrutinise rather than regenerate.

Established it was environmental, not the change:

- **The fixture's `.ir` is byte-identical to its golden.** It was not among the
  mismatches, and a direct `diff` confirms it. The generated code for this
  fixture did not change at all, so the change cannot be what crashed it.
- The same binary, run directly, passed **8/8**.
- Re-running the fixture through the harness UNCONTENDED passes against the
  unmodified golden.

The run was contended: a peer session's `test-accept` in the main checkout had
only just released the lock, and this fixture opens real TLS network
connections. Regenerating would have pinned a SIGSEGV as expected output —
a dead fixture, and the exact thing `no_golden_pins_a_fatal_signal` exists to
catch. The golden was left untouched and the sync was filtered to the 8
legitimate paths.

**D-C6 — a diagnostic pin changes MEANING, not just text.**
`syntax/json/func_json_stringify_invalid` deliberately called `stringify` with
two arguments to prove the arity was one. With the overloads that call has a
LEGAL arity and wrong types, so the diagnostic correctly moves from
`TYPE_CALL_ARITY_MISMATCH` to `TYPE_CALL_ARGUMENT_MISMATCH`:

```
< Call to `json.stringify` has 2 argument(s), expected 1.
> Call to `json.stringify` has argument type(s) (json.JsonNull, json.JsonNull),
>   expected Json or Json, Integer or Json, String.
```

The other two hunks are the arity range widening to "1 to 2" and the
`expected_arguments` hint naming all three forms. All three are this letter
working; the fixture still rejects the same source, which is what it is for.

**D-C3 — a compiler bug this letter exposed: adding an overload silently stops
union wrapping.** This is the letter's stated care point ("the overloads must
keep that acceptance") turning out to be a real defect rather than a thing to
verify. The moment `json::stringify` had more than one implementation, passing a
union MEMBER type to the 1-arg form produced **wrong output with no diagnostic**:

```
json::stringify(json::JsonNull[NOTHING])  ->  ""        (want "null")
json::stringify(json::JsonBool[TRUE])     ->  ""        (want "true")
json::stringify(json::JsonStr["Ada"])     ->  "null"    (want "\"Ada\"")
```

The tag was being read from the wrong place. Bisected to the overload set, not
the new helpers: with the helpers registered but only ONE implementation, all
three cases were correct.

Root cause: `argument_types_typed` (`registry/mod.rs`) returns `None` whenever
`implementations.len() > 1`, and `expected_parameter_type` in `ir/lower.rs`
consumes it **to decide union wrapping** — its own doc says so. With no expected
type, the member-type argument lowered as a bare record where the callee expects
a tagged union.

Fixed at the seam rather than worked around: a position is only ambiguous if the
overloads actually disagree there. New `agreed_argument_type(qualified, index)`
returns the type when EVERY implementation declares the same one at `index` —
correct regardless of which overload is later selected — and `None` where they
differ (`indent`: `Integer` vs `String`), leaving those to the existing path.
`expected_parameter_type` consults it after `argument_types_typed` declines.

Pinned by `agreed_argument_type_answers_where_overloads_agree`, which asserts
BOTH halves: position 0 answers `json.Json`, position 1 stays `None`. A test that
only checked the first half would let a future change guess at a disagreeing
position.

This was in scope: the plan named it, and AGENTS requires fixing a found bug
rather than routing around it. It also means any future overload added to a
member with a union-typed parameter would have hit the same silent corruption.

**D-C2 — the overload precedent is `crypto::hash`, not `process::spawn` or
`datetime::parse`.** References named `process::spawn` (and letter A's execution
corrected that to `datetime::parse`). Both are ARITY splits — 1-arg vs 4-arg,
2-arg vs 3-arg. This letter needs two overloads of the SAME arity distinguished
by parameter TYPE (`Integer` vs `String`), which neither demonstrates.

`crypto::hash` is the real precedent (`func_hash.rs:130-190`): `(Hash, List OF
Byte)` and `(Hash, String)`, same arity, selected by type. Confirmed working —
`match_overload` unifies each implementation's parameter types against the call's
argument types and takes the first that fits, so type dispatch at equal arity is
already supported.

One difference worth recording: `crypto::hash`'s second overload must use
`Body::Rewrite` because two `AbiFunction` overloads would collapse onto one
helper symbol. That constraint does not apply here — both of this letter's
overloads are `Body::mfb` with distinct function names
(`__json_stringifyCount`, `__json_stringifyText`), the `datetime::parse` shape.

**D-C4 — the string clamp must count SCALARS, because that is what it slices.**
Writing the truncation as "count graphemes, then `strings::mid(text, 0, 10)`"
looks right and is not: `mfb man strings mid` documents `start` and `count` as
**scalar** indices, and requires `start + count` not to exceed the scalar length —
and `strings::mid` raises rather than clamping (a known trap). A text of 11
graphemes can be 10 scalars, so a grapheme-counted guard would let such a text
through to a `mid` that raises.

Counting scalars (`len(encoding::utf32Encode(text))`) is self-consistent with the
slice AND closer to Node, which truncates by UTF-16 code unit — identical to
scalars for every BMP character, which is every indent anyone writes. Recorded
here because the plan's "first 10 characters" wording does not say which unit,
and the grapheme reading is the one that looks more MFBASIC-idiomatic.

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
