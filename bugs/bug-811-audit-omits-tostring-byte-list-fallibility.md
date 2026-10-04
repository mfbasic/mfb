# bug-811: `mfb audit` does not report `toString(List OF Byte)` as a fallible call

Last updated: 2026-10-04

Effort: small (<1h)

Severity: LOW

Class: Correctness

Status: Open
Regression Test: proposed `src/audit/collect/source.rs:tests::tostring_byte_list_is_fallible`

`toString` applied to a `List OF Byte` decodes UTF-8 and raises `ErrEncoding`
for invalid bytes (fixed to actually declare this in bug-807's codegen-side
census). `mfb audit`'s own, separate AST-level census
(`src/audit/collect/source.rs`) still reports a function whose only call is
`toString(<List OF Byte>)` as total (`fallible: false`), and emits no call row
for it — the same reporting-side gap bug-679 fixed for `toInt`'s enum overload,
but in the opposite direction: `toInt` is now correctly narrowed from fallible
to infallible for its one total overload, while `toString` is not widened from
infallible to fallible for its one fallible overload, because `is_fallible_builtin`
does not list `toString` by name at all.

References:

- `bugs/completed/bug-807-general-conversions-declare-no-errors.md`, "Not
  changed (open decision)": "`mfb audit`'s own name-keyed census
  (`audit/collect/source.rs:is_fallible_builtin`) does not list `toString` at
  all, so it reports `toString(<List OF Byte>)` as total — the same
  reporting-side gap bug-679 describes for `toInt`, left with that decision."
  This bug is that left-open gap, filed as its own ticket.
- `bugs/completed/bug-679-toint-enum-classified-fallible.md`, "Corrections §1"
  — establishes that `mfb audit` has its own independent AST census, separate
  from the IR-level `inline_builtin_is_infallible` that bug-807 fixed; fixing
  one does not fix the other.
- `src/audit/collect/source.rs:1011-1130` (`is_fallible_builtin`) — the
  name-keyed table; `toInt`, `toFloat`, `toFixed`, `toMoney`, `toScalar`,
  `toByte` are present, `toString` is absent.
- Found during bug-807 follow-up work (bug-staging-7) reproducing findings
  reported against `http`/`canvas`/`audio`/`csv`/`mfb audit`.

## Failing Reproduction

```sh
mkdir -p /tmp/mfb-f5/src
cat > /tmp/mfb-f5/project.json <<'JSON'
{"name": "f5", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "entry": "main", "targets": ["native"]}
JSON
cat > /tmp/mfb-f5/src/main.mfb <<'BASIC'
IMPORT io

FUNC decode(b AS List OF Byte) AS String
  RETURN toString(b)
END FUNC

FUNC main() AS Nothing
  io::print(decode([255, 254]))
END FUNC
BASIC
target/release/mfb audit --format json /tmp/mfb-f5
```

- Observed (macos-aarch64, `bug-staging-7` release build at
  `/tmp/bs7-target/release/mfb`): `sourceFlow` reports

  ```json
  {
    "function": "decode",
    "path": "src/main.mfb",
    "line": 3,
    "fallible": false,
    "trap": null,
    "calls": []
  }
  ```

  Running the program's `toString(b)` directly (per bug-807's own
  reproduction, `toString([255])`) raises `77020004` (`ErrEncoding`) at
  runtime — `decode` genuinely is fallible, and `mfb audit` says it is not.

- Expected: `decode` is reported `"fallible": true` with a call row for
  `toString`, the way the text report (`mfb audit /tmp/mfb-f5`) lists
  `io.print` as a fallible call today — and the same contrast `bug-679`'s
  table drew for `toInt`/`toString(<enum>)` applies here in reverse: `toInt`
  on a `String` is (correctly) reported fallible; `toString` on a
  `List OF Byte` is (incorrectly) reported total.

## Root Cause

`src/audit/collect/source.rs:is_fallible_call` dispatches a bare conversion
name to `is_fallible_builtin(callee)` (line ~1002). `is_fallible_builtin`
(line 1011) lists the bare `general` conversions that can fail by name:
`toInt`, `toFloat`, `toFixed`, `toMoney`, `toScalar`, `toByte` — and the
comment above that block explicitly frames it as "the bare `general`
conversions: a value that does not parse raises `ErrInvalidFormat` and one
outside range raises `ErrOverflow`". `toString` is not in that list at all,
so `is_fallible_call` falls through to `fallible.contains(callee)` (the
user-function fixpoint), which `toString` — being a builtin, not a declared
user function — never populates. The result is `false` for every
`toString` call regardless of argument type, including its one fallible
overload (`List OF Byte`, `ErrEncoding`).

This is the AST census's own gap, independent of the IR-level
`inline_builtin_is_infallible` (`src/codegen/builtins/mod.rs:399`) that
bug-807 and bug-486 already made overload-aware for `toString` on the
codegen/dead-handler side — `mfb audit` has never read that census, per
bug-679's Corrections §1.

## Goal

`mfb audit` reports a call to `toString` with a `List OF Byte`-typed argument
as fallible, with a call row naming it, exactly as the codegen-side census
and the man page's Errors table already do (post bug-807). A `toString` call
on any other argument type stays total, matching `toString`'s own
infallibility for those overloads.

### Non-goals (must NOT change)

- No change to `toInt`'s existing enum-aware rule (bug-679) or to any other
  entry already in `is_fallible_builtin`.
- No change to runtime behavior, codegen, or the IR-level
  `inline_builtin_is_infallible` census — this is a reporting-only fix in
  `mfb audit`'s AST census.
- No change to the inline-`TRAP` dead-handler warning behavior (that census is
  the one bug-807 already updated).

## Blast Radius

- `src/audit/collect/source.rs:is_fallible_call`/`is_fallible_builtin` — the
  fix site; needs the same byte-list-argument type inference bug-679 added for
  `toInt`'s enum detection (a per-function binding map, since the AST census
  has no type checker to consult).
- Every `mfb audit` report (text and JSON) of a program calling
  `toString` on a `List OF Byte` — measured reach not yet enumerated; left to
  Phase 1 (`grep -rn "toString(" tests/ examples/` filtered to `List OF Byte`
  arguments).

## Phases

### Phase 1 — failing test + measurement

Add `is_fallible_builtin`/`is_fallible_call` unit case(s) in
`src/audit/collect/source.rs` asserting `toString(<List OF Byte> binding)` is
fallible and every other `toString` overload stays total. Enumerate the
corpus of `toString(` calls on a `List OF Byte` in `tests/`/`examples/` to
know which goldens move.

### Phase 2 — the fix

Give the AST census the same declared-type/binding-map approach bug-679 added
for `toInt`'s enum rule, narrowed to recognize a `List OF Byte`-typed argument
to `toString`, and mark that call fallible.

### Phase 3 — validation

`cargo test --bin mfb` (the new unit case plus the full audit suite);
re-run the reproduction above and confirm `"fallible": true` with a
`toString` call row; any `tests/syntax/packages/audit-capabilities` golden
touching `toString(<List OF Byte>)` regenerated and diffed to confirm the
delta is exactly this fallibility flip.
