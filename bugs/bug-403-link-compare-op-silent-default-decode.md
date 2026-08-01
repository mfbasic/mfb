# bug-403: decoded LINK `Compare` operator string is never validated; a garbage op from a crafted `.mfp` silently compiles as `=`

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Footgun / decode-hardening gap (silent-default on untrusted IR)

Status: Open
Regression Test: tests/ — a binary_repr/verify unit test asserting a decoded
`IrLinkExpr::Compare { op: "GARBAGE", .. }` is rejected (decode or verify Err),
not accepted.

`decode_link_expr_body` (`src/ir/binary.rs:738`, tag 2) reads the
`IrLinkExpr::Compare` `op` as an arbitrary `r.string()?` with **no validation**
that it is one of `= <> < > <= >=`. The package-path verifier does not close the
gap either: `src/ir/verify/link.rs` validates only the operand `Var` names (via
`link_expr_var_names`, which drops `op` with `..`), never the operator string. At
codegen, `src/target/shared/code/link_thunk.rs:2049-2057` matches the op and falls
through to `_ => abi::branch_eq(&end)`:

```rust
let branch = match op.as_str() {
    "=" => abi::branch_eq(&end),
    "<>" => abi::branch_ne(&end),
    ...
    _ => abi::branch_eq(&end),   // any unknown op → silently treated as `=`
};
```

So a crafted `.mfp` whose `success_on`/`result` carries `Compare { op: "GARBAGE",
.. }` silently compiles as an `=` comparison instead of being rejected.

This directly contradicts the codebase's own stated invariant for the sibling
decoded field `AbiDirection` — "An unknown direction must be an error, never a
silent default" (`binary.rs:648-651` / `link.rs:826-843`, which IS validated via
`from_code → Err`). The `op` is the one decoded LINK-expr field with no equivalent
guard.

Impact is a silently-wrong comparison (not memory unsafety), reachable only via a
hand-crafted/malformed `.mfp` — the same threat model as the file's other
decode-hardening (PKG/sec items). The compiler's own `lower_link_expr`
(`lower_link.rs:307`) only ever emits valid ops, so it is unreachable from normal
builds. Hence LOW.

References:

- `src/ir/binary.rs:738` (decode, no op validation); `src/ir/verify/link.rs`
  (`link_expr_var_names` drops `op`); `src/target/shared/code/link_thunk.rs:2049-2057`
  (`_ => branch_eq` silent default).
- Sibling that IS validated: `AbiDirection::from_code` (`binary.rs:648-651`).
  Found during goal-07.

## Failing Reproduction

Static trace (decode → verify → codegen); no crafted `.mfp` fixture built (hand-
authoring a signed package with a malformed LINK-expr op is non-trivial). The
`_ => abi::branch_eq(&end)` fallthrough is present and unconditional for any op
outside the six recognized strings.

- Observed: `Compare { op: "GARBAGE" }` in a decoded package → compiles as `=`.
- Expected: an unknown operator string is a decode/verify error, matching
  `AbiDirection`.

## Root Cause

The `op` field is decoded as a free-form string and validated nowhere along
decode → verify; codegen's exhaustive-looking `match` has a permissive `_` arm
that defaults to `=` instead of being unreachable-after-validation.

## Goal

- A decoded `IrLinkExpr::Compare` with an operator outside `= <> < > <= >=` is
  rejected at decode (or verify) with an error, exactly like an unknown
  `AbiDirection`.

### Non-goals (must NOT change)

- The six valid operators and their codegen; no change to how the compiler's own
  emitter produces ops.

## Blast Radius

- `src/ir/binary.rs:738` (add op validation at decode) — primary fix.
- `src/ir/verify/link.rs` — optionally also validate `op` for defense in depth.
- `src/target/shared/code/link_thunk.rs:2056` — the `_ => branch_eq` arm should
  become unreachable once decode validates; keep it as a loud `unreachable!`/error
  rather than a silent `=`.
