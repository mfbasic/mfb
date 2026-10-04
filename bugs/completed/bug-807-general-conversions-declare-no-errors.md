# bug-807: `toString`/`toInt` declare no errors though their lowering raises four

Last updated: 2026-10-04
Effort: medium (1h–3h)
Severity: LOW
Class: Other (documentation correctness)

Status: FIXED (2026-10-04, bug-staging-7 worktree, commits bd9f56cb0, 407fc9e85, 473b6bf34, 2922395cd)
Regression Test: `src/codegen/builtins/tests/general_declared_errors.rs` (per-overload
lowering guard for every `general` member); `tests/runtime/rt_general_conversion_errors.rs`

`general::toString` and `general::toInt` — the bare-name conversions every program
uses — declare `errors: vec![]`, so `mfb man general toString` and `mfb man general
toInt` have no Errors table, and `toString`'s page says "`toString` never fails".
Their lowering raises four codes: `toString(<List OF Byte>)` decodes UTF-8 and raises
`ErrEncoding`; `toInt` raises `ErrInvalidFormat` and `ErrOverflow` from its `String`
parses and `ErrOverflow` from a `Float` outside the `Integer` range. `toInt`'s own
prose already names `ErrInvalidFormat`/`ErrOverflow`; only the table is missing.

**Correct behavior.** Each `general` member's descriptor `errors` equals the union of
what its overloads' lowering can raise, the page states which overload raises which
code, and a guard ties the two together the way `declared_errors_all.rs` and
`mfb_body_declared_errors.rs` do for every other package (bug-698).

References:

- bug-698 (`bugs/completed/bug-698-builtin-descriptors-omit-errors-their-lowering-raises.md`),
  Resolution: found this gap, carried `htmlUnescape`/`punycodeDecode`'s `ErrEncoding`
  through `DECLARED_EXTRA` instead, and left `toString`/`toInt` to this bug.
- `.ai/man-content.md` §2: "Every raisable error and the condition that raises it,
  consistent with the auto-derived Errors table."
- Spec `language/04_types.md` (Float finiteness): "passed as an argument" includes every
  built-in's argument.

## Failing Reproduction

From the repository root, after `cargo build --release` at `72abd60b5`:

```sh
target/release/mfb man general toString | grep -c '^Errors'   # 0
target/release/mfb man general toInt    | grep -c '^Errors'   # 0
target/release/mfb man general toString | grep 'never fails'  # "`toString` never fails."

mkdir -p /tmp/mfb-bug-807/src
cat > /tmp/mfb-bug-807/project.json <<'JSON'
{"name": "b807", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-bug-807/src/main.mfb <<'BASIC'
IMPORT io

FUNC s1(label AS String, b AS List OF Byte) AS Nothing
  io::print(label & " " & toString(b))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC i1(label AS String, t AS String) AS Nothing
  io::print(label & " " & toString(toInt(t)))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC i3(label AS String, x AS Float, y AS Float) AS Nothing
  io::print(label & " " & toString(toInt(x / y)))
  RETURN
TRAP(e)
  io::print(label & " raised " & toString(e.code))
  RETURN
END TRAP
END FUNC

FUNC p(label AS String, x AS Float, y AS Float) AS Nothing
  io::print(label & " " & toString(isNegative(x / y)))
END FUNC

FUNC main() AS Nothing
  s1("toString([255])", [255])
  i1("toInt(x)", "x")
  i1("toInt(99999999999999999999)", "99999999999999999999")
  i3("toInt(1.0e300 * 10.0)", 1.0e300, 0.1)
  i3("toInt(1.0 / 0.0)", 1.0, 0.0)
  p("isNegative(0.0 / 0.0)", 0.0, 0.0)
END FUNC
BASIC
target/release/mfb build /tmp/mfb-bug-807 && /tmp/mfb-bug-807/build/b807.out
```

- Observed:

  ```
  toString([255]) raised 77020004
  toInt(x) raised 77050003
  toInt(99999999999999999999) raised 77050010
  toInt(1.0e300 * 10.0) raised 77050010
  toInt(1.0 / 0.0) raised 77050003
  isNegative(0.0 / 0.0) TRUE
  ```

- Expected: the first four raise exactly what the pages' Errors tables list
  (`ErrEncoding`, `ErrInvalidFormat`, `ErrOverflow`, `ErrOverflow`), and the last two
  fail at the call with `ErrFloatOverflow` (`77050015`) / `ErrFloatNaN` (`77050013`),
  the observation-boundary codes `toString(1.0 / 0.0)` and `math::abs(1.0 / 0.0)`
  already raise.

## Root Cause

The `general` members are `Body::Intrinsic`: the registry entry is a home for
`declares_error("general.toX", …)` and the man page, and codegen is the builder's own
bare-name dispatch (`builder_conversions.rs:lower_to_int`,
`builder_strings.rs:lower_to_string`). `toInt` raises through `raise_error_bare`, which
validates nothing against the descriptor, and `toString`'s `ErrEncoding` is emitted by
`emit_byte_list_to_string_value`. Neither guard bug-698 built could see them:
`declared_errors_all.rs` lowers only `AbiFunction`/`AbiInline` bodies, and
`mfb_body_declared_errors.rs` scans `Body::Mfb` source for `FAIL`s and *qualified*
`pkg::name(` / `__name(` calls — a bare `toString(` matches neither, so every
package body that calls a conversion propagated nothing from it.

The sibling defect is the same seam one level down: `lower_to_string` calls
`observe_float` on its argument (the spec's observation boundary), and `lower_to_int`,
`lower_to_fixed`, `lower_to_money` and `lower_numeric_filter_predicate` do not, so an
arithmetic infinity or NaN reaches the conversion.

## Goal

- Every `general` member's descriptor `errors` equals the union of its overloads'
  measured raise sets, guarded per overload on all five backends.
- Each page states which overload raises which code; `toString` no longer says it
  never fails.
- A `Float` argument of `toInt`/`toFixed`/`toMoney`/`isZero`/`isPositive`/`isNegative`
  is observed at the call.
- The `Body::Mfb` guard sees bare `general` calls, and every package member they make
  reach a new code is resolved: the code declared where reachable, or proven
  unreachable with a `(package, member, code)` or `(FUNC, member, code)` entry.

### Non-goals (must NOT change)

- The inline-`TRAP` infallibility census (`builtins::inline_builtin_is_infallible`)
  keeps `toString`'s non-`List OF Byte` verdict; see Open Decisions.
- No change to what any valid conversion returns.

## Blast Radius

- `general::toString`, `toInt`, and every other `general` member (the guard covers all
  18; `error` turns out to raise `ErrOutOfMemory` as well).
- Every `Body::Mfb` member whose body or private helper calls a bare conversion —
  measured in Phase 1.
- `toInt`/`toFixed`/`toMoney`/`isZero`/`isPositive`/`isNegative` codegen for an
  arithmetic `Float` argument (one finiteness check, as `toString` already has).

## Phases

### Phase 1 — failing guard + measurement

`general_declared_errors.rs`: compile one `probe` per overload at `-O0` on all five
backends, collect every error code `probe` and its internal callees load (the shared
`RAISES_NOTHING` helpers excluded, as in `declared_errors_all.rs`), assert the
per-overload rows and the per-member union against the descriptor. Teach the
`Body::Mfb` guard to propagate bare general calls by overload.

### Phase 2 — the fix

Fill the descriptors, align the prose, observe the `Float` argument, and resolve every
member the extended `Body::Mfb` guard flags.

### Phase 3 — validation

The guards (`cargo test --release --bin mfb declared_errors`), the runtime tests, the
rendered pages, `scripts/man-census.sh`, targeted `scripts/test-accept.sh` fixtures.

## Open Decisions

- The inline-`TRAP` census treats `toString` on any argument but `List OF Byte` as
  infallible by name, so a handler on `toString(42) TRAP(e)` is dead and warned
  `TYPE_INLINE_TRAP_DEAD_HANDLER`, while the allocating `collections` members, which
  also declare only `ErrOutOfMemory`, keep a live handler. Making `toString` fallible
  there changes the function-fallibility fixpoint and the short-circuit shape rule for
  every program that traps an expression containing `toString`. Left to the owner.

## Resolution (2026-10-04)

**Measured raise sets.** `general_declared_errors.rs` lowers one call per overload
(69 rows, all 18 `general` members) at `-O0` on all five backends and scans `probe`
plus its internal callees. Before this bug only `toString`, `toInt` and `error`
disagreed with their descriptors:

| member | overload | raises (measured) |
| --- | --- | --- |
| `toString` | every form | `ErrOutOfMemory` |
| `toString` | `List OF Byte` | `ErrEncoding`, `ErrOutOfMemory` |
| `toInt` | `String`, `(String, Integer)` | `ErrInvalidFormat`, `ErrOverflow` |
| `toInt` | `Float` | `ErrOverflow` |
| `toInt` | `Fixed`, `Money`, `Byte`, `Scalar`, enum | — |
| `error` | `(Integer, String)` | `ErrOutOfMemory` |

Four loaded-but-dead codes are proven in the guard's `UNREACHABLE`: the `Float`
NaN/infinity exits of `toInt`/`toFixed`/`toMoney` (the argument is finite once
observed, below), `toFixed(Money)`'s divisor check (constant 100000) and
`toMoney(Fixed)`'s range check (a Fixed is below 2^31, the product below 2^48).

**Commits.**

- `bd9f56cb0` — `toInt`/`toFixed`/`toMoney`/`isZero`/`isPositive`/`isNegative` call
  `observe_float` on their argument, as `toString` did. `toInt(1.0 / 0.0)` now raises
  77050015 (was 77050003) and `isNegative(0.0 / 0.0)` 77050013 (was `TRUE`).
- `407fc9e85` — the guard; `toString`/`toInt`/`error` descriptors and prose; `toInt`,
  `toFixed`, `toMoney` prose for which forms fail; spec `18_builtin-functions`
  (per-overload raise sets) and `08_error-model` (the census note below).
- `473b6bf34` — `__canvas_readDisplayOverride` ignores a malformed or out-of-range
  `MFB_CANVAS_DISPLAY_HZ` instead of raising `ErrInvalidFormat`/`ErrOverflow` out of
  `canvas::present`/`presentLayers`/`getFrameStats`/`waitFrame`/`setBytes`.
- `2922395cd` — `mfb_body_declared_errors.rs` propagates bare general calls by
  overload (its own type inference from the body text; an untyped argument counts
  every overload), resolves each `Body::Mfb` string one `FUNC` at a time, and adds
  `UNREACHABLE_GENERAL_CALL` (`(FUNC, member, code)`, 35 range proofs). Naive
  member-level propagation flagged 97 members; inference left 48. Real raises now
  declared: `ErrOutOfMemory` on `encoding::utf16Decode`/`utf32Decode`/`codepageDecode`
  and `datetime::fixedOffset`/`parseIso` (codepageDecode's old OOM-unreachable entry
  was wrong: `__encoding_fromCodepoint` allocates through `toString`). The
  `htmlUnescape`/`punycodeDecode` `ErrEncoding` `DECLARED_EXTRA` entries are gone: the
  guard sees the raise itself.

**Verification.**

```
CARGO_TARGET_DIR=/tmp/b807-target cargo test --release --bin mfb declared_errors
# 24 passed; 0 failed (adds every_general_member_declares_exactly_the_errors_its_lowering_raises)
MFB_TEST_EXE=/tmp/b807-target/release/mfb CARGO_TARGET_DIR=/tmp/b807-target \
  cargo test --release --test rt_general_conversion_errors --test rt_canvas_frame_stats
# 2 passed; 12 passed (RED on the bug-staging-7 build: float observation, display override)
MFB=/tmp/b807-target/release/mfb ./scripts/man-census.sh --memory-scope general datetime csv encoding canvas
# unclassified memory-vocabulary hits: 0
MFB=/tmp/b807-target/release/mfb ./scripts/man-census.sh --scope general datetime csv encoding canvas
# internals-vocabulary hits: 0
MFB=/tmp/b807-target/release/mfb ./scripts/man-run-examples.sh general --run
# examples: 36 built: 36 ran: 36 failed: 0
```

**Expected golden moves (not regenerated here).** `.ir` of every fixture that imports
`datetime` (the format helper's month check moves later lines) or `csv` (readRow's
checks) — 23 of the 79 `*/datetime/*`, `*/csv/*`, `*/general/*` fixtures, `.ir` only,
every `.run`/`build.log` unchanged; any `.ir`/`.ncode` that injects `canvas`'s
`helper_render`; `.ncode`/byte-identity output of a program passing an arithmetic
`Float` straight to `toInt`/`toFixed`/`toMoney`/`isZero`/`isPositive`/`isNegative`
(one finiteness check); rendered `mfb man` output for the touched pages.

**Not changed (open decision).** The inline-`TRAP` census still calls `toString` on a
non-`List OF Byte` argument infallible by name, although it now declares
`ErrOutOfMemory`; see Open Decisions. `mfb audit`'s own name-keyed census
(`audit/collect/source.rs:is_fallible_builtin`) does not list `toString` at all, so it
reports `toString(<List OF Byte>)` as total — the same reporting-side gap bug-679
describes for `toInt`, left with that decision.
