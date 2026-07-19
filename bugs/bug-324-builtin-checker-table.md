# bug-324: 22 near-identical `check_<pkg>_builtin_call` methods + a 236-line hand-written dispatcher in `src/syntaxcheck/builtins.rs`

Last updated: 2026-07-18
Effort: medium-to-large
Severity: LOW
Class: Other-cleanup

Status: Open
Regression Test: acceptance suite (`scripts/test-accept.sh`) — 522 `tests/syntax/**/golden/build.log`
fixtures pin builtin-call diagnostics verbatim

`src/syntaxcheck/builtins.rs` checks builtin calls with 22 per-package methods
(`check_vector_builtin_call` … `check_collections_builtin_call`) plus a hand-written
`check_builtin_call` dispatcher that is 236 lines of copy-pasted `if
builtins::<pkg>::is_<pkg>_call(callee) { return self.check_<pkg>_builtin_call(…); }`
arms. Measured: the 22 methods are 1,438 lines and the dispatcher is 236, so
**1,674 of the file's 2,088 production lines (80%) are this one pattern** — in a
3,090-line file whose remaining 1,001 lines are its inline test module.

Nine of the 22 methods are identical after normalizing only the module path and
`rustfmt` line-wrapping. Two more are identical apart from one `ExprMode` enum value;
one more apart from two local variable names; five more apart from a small
argument-ownership policy block. All 22 packages expose the same four-function API in
`src/builtins/*.rs` (`is_<pkg>_call`, `arity`, `resolve_call`, `expected_arguments`),
so 18 of the 22 collapse to one generic method driven by an 18-row `BuiltinPackage`
table. Four are genuinely bespoke and must stay.

The single correct outcome a fix produces: **the same diagnostics, byte for byte**,
from ~150 lines instead of ~1,290. This is a maintenance-cost bug, not a behavior bug
— but the duplication is already load-bearing, because two of the five
ownership-policy packages hardcode a rule the other three read from a shared
`consumes_argument` predicate (see Blast Radius), which is exactly how these copies
drift.

References:

- `src/docs/spec/architecture/02_frontend.md` — syntaxcheck's role and the plan-20
  relocation boundary.
- Cleanup review 2026-07-18, Agent 13 #1 / #16 and Agent 17 #1 (converging findings).
- Related: bug-325 (plan-20 relocation residue in the same module); bug-322
  (arena-alloc boilerplate) for the same "one shape, N hand-written copies" class.

## Current State

Measured on `src/syntaxcheck/builtins.rs` (3,090 lines; production 1–2,088; inline
`mod tests` 2,090–3,090) by extracting each method by brace matching, substituting the
package name out of `check_<pkg>_builtin_call` / `builtins::<pkg>::` / the
`` `<pkg>. `` diagnostic-string prefix, collapsing whitespace, and hashing.

| Group | Packages | Sites | Lines | Delta from the canonical body |
| --- | --- | --- | --- | --- |
| A | json, csv, regex, datetime, io, money, strings, math, bits | 9 | 56 ea. (io 55) | **none** — identical modulo module path + line-wrap |
| B | vector, os | 2 | 56, 55 | `ExprMode::Borrow` instead of `ExprMode::Read` (1 line) |
| C | crypto | 1 | 56 | two local variable *names* only (4 lines) |
| D | fs, net, tls, audio, http | 5 | 61, 63, 63, 64, 64 | a per-argument-index ownership-mode block (~8 lines) |
| E | encoding | 1 | 71 | group C's naming + a 15-line `utf8Encode` contextual-return-type tail |
| — | **term, thread, general, collections** | 4 | 74, 100, 84, 124 | genuinely different — see Non-goals |

Totals: 22 methods = 1,438 lines (`builtins.rs:243`–`:1711`); dispatcher
`check_builtin_call` = `builtins.rs:5`–`:240` (236 lines); table-able groups A–E =
18 packages / 1,056 lines.

Correction to the review lead: the identical group is **nine, not eight** — `io`
(`builtins.rs:904-958`) belongs to it. It was missed because naive substring
normalization of the package name `io` also rewrites the substring inside
`infer_expression`, manufacturing a false difference. Likewise the near-identical set
is **nine packages differing by 1–13 lines** (vector, os, crypto, fs, net, tls, audio,
http, encoding), not seven.

Verbatim canonical body — `src/syntaxcheck/builtins.rs:611-666`
(`check_json_builtin_call`), reproduced character-for-character by eight sibling
methods after the module path is substituted:

```rust
    pub(super) fn check_json_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::json::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::json::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::json::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }
```

### The crypto naming outlier (Agent 13 #16)

`check_crypto_builtin_call` (`src/syntaxcheck/builtins.rs:1371-1426`) is the canonical
body with two locals renamed: `expected` → `expected_count` at `:1392` (used at
`:1397`) and `expected` → `expected_args` at `:1403` (used at `:1410`). Nothing else
differs — the emitted rule names, message templates, and control flow are identical.

This is why crypto did not cluster with the nine: the local names are *only* visible in
the source, never in output, so a text-similarity pass files crypto as a variant when
it is a verbatim clone. `encoding` (`:1428-1498`) inherits the same two renames, which
is why it too reads as further from the canonical body than it is — its only real
divergence is the trailing `utf8Encode` block at `:1483-1495`, and its extra `expected:
Option<&Type>` parameter.

### The dispatcher

`check_builtin_call` (`src/syntaxcheck/builtins.rs:5-240`) is 22 hand-ordered arms of
the same shape. Two consequences beyond length:

- Order is load-bearing and undocumented (`builtins::collections::is_native_member_call`
  must be consulted before the general arm), with nothing asserting the packages are
  mutually exclusive.
- Both the dispatcher (`:5`) and `check_encoding_builtin_call` (`:1428`) carry
  `clippy::too_many_arguments` warnings (8/7) solely because `expected: Option<&Type>`
  is threaded to a single consumer.

## Root Cause

`src/builtins/mod.rs` already establishes a uniform per-package API — every one of the
22 modules exposes `pub(crate) fn is_<pkg>_call`, `arity`, `resolve_call`, and
`expected_arguments` (verified: all 22 have exactly one of each). But
`src/syntaxcheck/builtins.rs` consumes that uniform API through *concrete module paths*
(`builtins::json::arity`) rather than through a value, so Rust's module system offers
no way to write the body once. Each new builtin package was therefore added by copying
the previous package's method and its dispatcher arm — the mechanical consequence of
having a uniform interface with no first-class representation.

The four bespoke methods are bespoke for real reasons, not drift:

- `check_term_builtin_call` (`:1017-1090`) — `builtins::term` is the only package that
  derives arity and `expected_arguments` from a single `param_types` table, so its
  `resolve_call` takes one argument and `expected_arguments` returns `String`, not
  `&'static str`. It checks arguments positionally against `param_types` via
  `expression_compatible` (`:1065`) rather than by joined type-name string.
- `check_thread_builtin_call` (`:1099-1198`, 100 lines), `check_general_builtin_call`
  (`:1500-1583`, 84 lines), `check_collections_builtin_call` (`:1588-1711`, 124 lines)
  — each carries package-specific typing logic on top of the common shape.

## Goal

- The nine identical + nine near-identical package checkers are replaced by one generic
  method driven by a declarative `BuiltinPackage` table with one row per package.
- `check_builtin_call` iterates that table instead of listing 22 hand-written arms,
  with the collections-before-general precedence made explicit in the row order and
  asserted by a test.
- `cargo clippy` no longer reports `too_many_arguments` at `builtins.rs:5` and `:1428`.
- Every `tests/syntax/**/golden/build.log` is unchanged — zero golden churn.

### Non-goals (must NOT change)

- **Diagnostics output must not change.** No rule name, message template, argument
  ordering, span, or emission order may shift. `scripts/test-accept.sh` compares 522
  `tests/syntax/**/golden/build.log` files verbatim; a passing run with zero golden
  regeneration is the acceptance bar. Regenerating a golden to accommodate a
  refactor-introduced wording change is explicitly forbidden.
- The four bespoke checkers — `term`, `thread`, `general`, `collections` — stay
  bespoke. Do not force them into the table; the table exists to remove *copies*, not
  to invent a universal abstraction. `term_return_type` (`:1091-1097`) stays with them.
- `src/builtins/*.rs` public surfaces (`arity` / `resolve_call` /
  `expected_arguments` / `is_<pkg>_call` signatures and semantics).
- The relative order in which packages are consulted by the dispatcher.
- `encoding`'s `utf8Encode` contextual-return-type behavior (`:1483-1495`) and the
  `expected: Option<&Type>` it needs.

## Blast Radius

Found by extracting and hashing every `check_*_builtin_call` body and grepping
`src/builtins/` for the shared API.

- `src/syntaxcheck/builtins.rs:611,668,790,847,904,960,1200,1257,1314` (json, csv,
  regex, datetime, io, money, strings, math, bits) — group A, byte-identical; fixed by
  this bug, table rows with `ExprMode::Read`.
- `src/syntaxcheck/builtins.rs:243,362` (vector, os) — group B; fixed, table rows with
  `ExprMode::Borrow`.
- `src/syntaxcheck/builtins.rs:1371` (crypto) — group C; fixed, an ordinary
  `ExprMode::Read` row. The two renamed locals vanish with the copies.
- `src/syntaxcheck/builtins.rs:300,418,482,546,725` (fs, net, tls, audio, http) —
  group D; fixed by a per-row ownership-mode hook. **Note the live inconsistency this
  exposes:** `tls` (`:492`), `audio` (`:557`), and `http` (`:737`) call
  `builtins::<pkg>::consumes_argument(callee, index)` (defined at
  `src/builtins/tls.rs:201`, `audio.rs:402`, `http.rs:271`), while `net` (`:428`) and
  `fs` (`:310`) hardcode `callee == "<pkg>.close" && index == 0` inline. There is no
  `consumes_argument` in `src/builtins/net.rs` or `src/builtins/fs.rs`. Two copies of
  one rule, one of them invisible to the package module that owns the fact.
- `src/syntaxcheck/builtins.rs:1428` (encoding) — group E; fixed by a table row plus a
  per-row post-resolve hook for the `utf8Encode` contextual type.
- `src/syntaxcheck/builtins.rs:5-240` (`check_builtin_call`) — fixed; becomes a table
  walk plus four bespoke arms.
- `src/syntaxcheck/builtins.rs:1017,1099,1500,1588` (term, thread, general,
  collections) — **out of scope**, keep as written; only their dispatcher arms move.
- `src/syntaxcheck/builtins.rs:2090-3090` (inline `mod tests`) — ~95 per-package
  builtin tests of one shape; unaffected by this bug and left alone. Collapsing them
  to a table-driven test is a separate cleanup (cleanup review Agent 13 #13).
- `src/builtins/mod.rs:305-327,331-350,377-398,514-535` — four *other* hand-maintained
  per-package dispatch chains with the same root cause. Same hazard, NOT addressed
  here — out of scope because they are in a different crate module with different
  consumers, and doing both at once makes the golden diff unreviewable. Worth a
  follow-up once `BuiltinPackage` exists.

## Fix Design

Introduce a `BuiltinPackage` descriptor in `src/syntaxcheck/builtins.rs` (or
`src/builtins/mod.rs` if the table is to be shared with the four chains above — see
Open Decisions) holding function pointers to the uniform API plus the two policy
fields the measured diff actually requires:

```
struct BuiltinPackage {
    is_call: fn(&str) -> bool,
    arity: fn(&str) -> Option<(usize, usize)>,
    resolve_call: fn(&str, &[String]) -> Option<ResolvedCall<'_>>,
    expected_arguments: fn(&str) -> Option<&'static str>,
    /// Group A/C = Read, group B = Borrow, group D = per-index.
    arg_mode: ArgMode,
}
```

`ArgMode` is the only semantic axis: `Read`, `Borrow`, or
`Consuming { consumes: fn(&str, usize) -> bool, default: ExprMode }`. Give
`src/builtins/net.rs` and `src/builtins/fs.rs` the `consumes_argument` predicate their
three siblings already have, so all five group-D rows are uniform and the ownership
rule lives once, in the package module that owns it. That change is behavior-preserving
by construction: the new predicates return exactly what the inlined conditions
returned.

`encoding`'s tail becomes an optional per-row `post_resolve: Option<fn(…)>` hook, or —
simpler — the table method returns the resolved type and `check_builtin_call` applies
the `utf8Encode` contextual override in the one arm that needs it, keeping
`expected: Option<&Type>` off the generic path entirely. Prefer the latter; it also
retires the `too_many_arguments` warning at `:1428`.

Where the risk concentrates: the generic body must reproduce the arity-mismatch
early-return and the argument-mismatch `else`-arm *in the same order*, because
`self.report` appends to a source-ordered `diagnostics` vector and `infer_expression`
itself reports nested errors as a side effect. Any reordering of the
`infer_expression` loop relative to the arity check changes diagnostic ordering — that
is the one way this refactor can move a golden. Land group A alone first and confirm
zero golden churn before extending to B–E.

Rejected alternatives:

- **A `trait BuiltinPackage` with 22 unit-struct impls.** Same line count as today,
  moved; the duplication is data, not behavior, so it wants a table, not a trait.
- **A declarative macro emitting the 22 methods.** Removes the source lines but keeps
  22 monomorphized copies and makes the group B/C/D deltas macro parameters — the
  divergences become harder to see, not easier, and grep stops finding the checkers.
- **Folding term/thread/general/collections in via escape-hatch hooks.** The hooks
  would be as long as the bespoke bodies and would make the common path conditional on
  four one-off flags.

## Phases

### Phase 1 — baseline + audit (no behavior change)

- [ ] Record a clean `scripts/test-accept.sh` run as the byte-exact baseline; note the
      golden count so a silently-skipped fixture is detectable.
- [ ] Add a table-driven unit test asserting that for every package, exactly one
      `is_<pkg>_call` returns true for each builtin callee name (proves the dispatcher
      arms are mutually exclusive and that order is not load-bearing beyond
      collections-before-general).
- [ ] Confirm the Blast Radius verdicts above still hold at head.

Acceptance: baseline recorded; the mutual-exclusion test passes (or its failures are
documented as the precedence constraint the table must preserve).
Commit: —

### Phase 2 — the table, group A only

- [ ] Add `BuiltinPackage` + `ArgMode` and one generic `check_table_builtin_call`.
- [ ] Convert the nine group-A packages; delete their nine methods and nine dispatcher
      arms.
- [ ] Run `scripts/test-accept.sh` — **zero golden diffs required** before proceeding.

Acceptance: acceptance suite green with no regenerated goldens; ~500 lines removed.
Commit: —

### Phase 3 — groups B, C, D, E + dispatcher

- [ ] Convert vector, os (B) and crypto (C).
- [ ] Add `consumes_argument` to `src/builtins/net.rs` and `src/builtins/fs.rs`; convert
      fs, net, tls, audio, http (D).
- [ ] Convert encoding (E), moving the `utf8Encode` contextual override into the
      dispatcher so the generic path drops `expected: Option<&Type>`.
- [ ] Replace the remaining dispatcher arms with a table walk plus the four bespoke
      arms.
- [ ] `cargo clippy` — confirm `too_many_arguments` is gone at `builtins.rs:5`/`:1428`
      and no new warnings appear.

Acceptance: acceptance suite green with no regenerated goldens; the file's production
half is ~800 lines; `term`/`thread`/`general`/`collections` untouched.
Commit: —

## Validation Plan

- Regression test(s): `scripts/test-accept.sh` in full — 522 `tests/syntax/**/golden/
  build.log` fixtures pin the exact diagnostic text for builtin arity and
  argument-type mismatches across every package. Plus the Phase 1 mutual-exclusion
  unit test in `src/syntaxcheck/builtins.rs`'s inline test module.
- Runtime proof: build a fixture per group with a deliberate arity error and a
  deliberate argument-type error (e.g. `json.parse()` and `net.close(1)`), and diff
  `mfb build` output before and after the refactor — must be byte-identical.
- Doc sync: none expected. No rule code, spec rule list, or man page changes; the
  refactor is invisible outside `src/syntaxcheck/builtins.rs` except for two new
  `consumes_argument` functions in `src/builtins/{net,fs}.rs`.
- Full suite: `cargo test` + `scripts/test-accept.sh` + `cargo clippy` (warning count
  must drop by at least 2, never rise).

## Open Decisions

- Where `BuiltinPackage` lives — `src/syntaxcheck/builtins.rs` (recommended: keeps the
  blast radius to one file) vs. `src/builtins/mod.rs` (enables reusing the same table
  to collapse the four dispatch chains at `mod.rs:305-327,331-350,377-398,514-535`,
  but widens this bug into a second module). Recommend starting local and promoting
  it in a follow-up once it has proven itself.
- Whether `encoding` gets a table row with a post-resolve hook or stays a bespoke
  fifth arm. Recommend the table row: the hook is 15 lines and the rest of the body is
  a verbatim clone.

## Summary

The engineering risk is not the table — it is proving the collapse is diagnostically
inert. `self.report` writes into a source-ordered diagnostics vector and
`infer_expression` reports nested errors as a side effect, so the generic body must
preserve the exact interleaving of argument inference, the arity early-return, and the
resolve-failure arm. The 522 syntax goldens make that provable rather than
argued: land group A first, demand a zero-diff acceptance run, and only then extend.
Left untouched: the four genuinely bespoke checkers, every `src/builtins/*.rs`
signature, and all emitted diagnostics.
