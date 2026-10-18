# bug-863: `crypto::open(cipher, key, nonce, sealed, aad)` — the documented Sealed+aad overload fails to compile with an unlocated TYPE_CALL_ARGUMENT_MISMATCH

Last updated: 2026-10-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: tests/rt-behavior/crypto/crypto-open-sealed-aad-rt; crypto::tests::open_pads_the_overload_its_arguments_select

## Cause (coordinator, bug-staging-24)

- Rule: the defaults appended to a call are those of the overload the call resolves to: default padding is part of overload resolution, decided by every provided argument's type, not the first.
- Where: `src/codegen/registry/mod.rs:default_argument_padding` picks the overload whose trailing `Fill` defaults to append by first-argument type only (`param.ty.name() == argument.name()`), so `crypto::open`'s 5-argument Sealed+aad call is padded with the ct/tag overload's `aad`.
- Fix: padding selects the overload whose leading parameters accept ALL provided arguments (the same assignability the resolver uses), then pads its defaults; the `crypto::open`-specific reasoning in its comment goes; a padding mismatch reports with the call's span.

`crypto::open` overload 2 is `open(cipher AS SymmetricCipher, key AS List OF Byte, nonce AS
List OF Byte, sealed AS crypto::Sealed, [aad AS List OF Byte])`. Calling it **with** the
optional `aad` (the only way to open a box sealed with associated data through the
`Sealed` record) does not compile. The compiler pads the call to six arguments, then
rejects it, and the diagnostic has no file/line.

Correct behavior: the call compiles and returns the plaintext (or raises
`ErrAuthenticationFailed` on a wrong `aad`), exactly like the explicit
`open(cipher, key, nonce, box.ciphertext, box.tag, aad)` form.

References:

- src/docs/spec/stdlib/10_crypto.md §AEAD: "`aad` defaults to empty"
- `mfb man crypto open`, Overloads: "2. `crypto::open(cipher AS crypto::SymmetricCipher,
  key AS List OF Byte, nonce AS List OF Byte, sealed AS crypto::Sealed, [aad AS List OF Byte])`"

## Failing Reproduction

`/tmp/hardening/B04/openaad/src/main.mfb`:

```
IMPORT io
IMPORT crypto
IMPORT strings

FUNC main() AS Nothing
  LET k AS List OF Byte = crypto::randomBytes(32)
  LET n AS List OF Byte = crypto::randomBytes(12)
  LET aad AS List OF Byte = strings::toBytes("hdr")
  LET box AS crypto::Sealed = crypto::seal(crypto::SymmetricCipher.AES256GCM, k, n, "hi", aad)
  LET clear AS List OF Byte = crypto::open(crypto::SymmetricCipher.AES256GCM, k, n, box, aad)
  io::print(toString(clear))
END FUNC
```

```
/tmp/hardening/mfb build /tmp/hardening/B04/openaad
```

- Observed (exit 1, no source location):
  ```
  error: TYPE_CALL_ARGUMENT_MISMATCH: Call to `crypto.open` has argument type(s) (crypto.SymmetricCipher, List OF Byte, List OF Byte, crypto.Sealed, List OF Byte, List OF Byte), expected crypto::SymmetricCipher, List OF Byte, List OF Byte, (List OF Byte, List OF Byte or crypto::Sealed)[, List OF Byte].
  ```
- Expected: builds; prints `hi`.

Contrast (all work, from `/tmp/hardening/B04/crx`): `open(cipher, k, n, sealed)` (4 args);
`open(cipher, k, n, ct, tag)` and `open(cipher, k, n, ct, tag, aad)`. Every AES-GCM and
ChaCha20-Poly1305 seal/open vector matched python `cryptography` 49.0.0.

## Root Cause

`src/codegen/registry/mod.rs:default_argument_padding` chooses which overload's trailing
`Fill` defaults to append by **first-argument type only**
(`param.ty.name() == argument.name()`) among overloads with
`params.len() >= provided`. Both `crypto::open` overloads start with `SymmetricCipher` and
both have ≥ 5 params. For the 5-argument Sealed+aad call it picks overload 1 (ciphertext,
tag, aad: 6 params) and appends its `aad` default, giving 6 arguments with a `Sealed` in
position 4. No overload accepts that. The function's own comment names this exact member
("`crypto::open`'s AEAD form has an exact-arity sibling at 5 arguments whose `aad` must
still be padded"), so it handles the ct/tag form and breaks the Sealed form. The selection
needs all provided argument types (or the overload the type checker actually selected), not
just the first. The missing location is a second defect: this padding-induced mismatch is
reported without the call's span.

## Goal

- The repro builds and prints `hi`; a wrong `aad` raises 77050016.
- The 5-arg `open(cipher, k, n, ct, tag)` call is still padded with empty `aad`.
- The diagnostic for any padding mismatch carries file:line.

### Non-goals (must NOT change)

- `tls::connect` Address/host padding (plan-110-D) keeps working.

## Blast Radius

- `default_argument_padding` serves every registry member with optional trailing params and ≥ 2 overloads sharing a first-parameter type. Grep descriptors for `DefaultValue::Fill` on multi-overload members; `crypto::seal` (List OF Byte vs String data, both 5 params) also shares a first type but its overloads have equal shape, so they are unaffected (verified: `seal(..., aad)` with String data compiles).

## Fix (bs24-agent-f, 2026-10-18)

**Fix.** `default_argument_padding(qualified, arg_types)` now takes every provided
argument's type (`Unknown` where lowering cannot type one) and pads the trailing
`Fill` defaults of the overload the resolver selects for them: strict
`RegistryFunction::resolve` first, then lenient `dispatch`, exactly the order
`resolved_return_type`/`rewrite_target` use. The first-argument spelled-name match and
its `crypto::open` comment are gone. `ir::lower` passes the types of the normalized
arguments (extended with `Unknown` for an argument an earlier arm already appended).
A call no overload accepts is padded with nothing, so `ir::shape` reports the call as
written, at its span (measured: `open(cipher, k, n, box, 7)` reports
`main.mfb:6 TYPE_CALL_ARGUMENT_MISMATCH`). The unlocated diagnostic came only from
the over-padded six-argument call, which can no longer be produced.

**RED → GREEN.** `test-accept 'rt-behavior/crypto/crypto-open-sealed-aad-rt'`: main's
compiler fails to build (the unlocated six-argument mismatch); this branch prints
`hi` (Sealed + aad), `hi` (explicit ct/tag/aad), `yo`, `yo` (both 4/5-argument forms
padded with empty aad), and `RAISED 77050016` for a wrong aad. Unit test
`crypto::tests::open_pads_the_overload_its_arguments_select` (Sealed+aad pads
nothing, Sealed pads one `List OF Byte`, ct/tag pads one).

**Spec.** `mfb spec language functions` gains the omitted-trailing-parameter rule;
`mfb spec stdlib crypto` says `aad` defaults in both `open` forms.

**Goldens expected to move.** None: a call some overload accepts is padded by that
overload, which is the old first-argument choice for every call that compiled before
(a different choice produced an argument count or type no overload accepts).

**Found, not fixed (pre-existing, not this bug):** the `seal`/`open` program leaks
on main too — without the Sealed+aad calls, main's compiler (`--debug`) ends with
857 allocs / 844 frees / 4592 B live.
