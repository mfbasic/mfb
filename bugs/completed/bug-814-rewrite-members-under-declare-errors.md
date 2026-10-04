# bug-814: 19 of 29 `Body::Rewrite` built-ins declare fewer errors than their bodies raise

Last updated: 2026-10-04
Effort: large (3h–1d) (guess, by analogy with bug-698 Phase 2)
Severity: LOW
Class: Correctness (documentation / error contracts)

Status: Fixed (2026-10-04, bug-814 worktree off bug-staging-9)
Regression Test: `src/codegen/builtins/tests/mfb_body_declared_errors.rs:every_body_rewrite_implementation_declares_exactly_the_errors_its_target_can_raise`;
`tests/runtime/rt_crypto_random_int_span.rs`;
`src/audit/collect/source.rs:tests::every_builtin_is_fallible_exactly_when_it_declares_an_error`

Found while fixing bug-811 (bug-staging-8). Owner decision (bug-698, 2026-09-25): every
function must list every error it can raise.

## Failing Reproduction

No guard checks a `Body::Rewrite` member's declared errors against its body:
- `declared_errors_all.rs` covers `AbiFunction`/`AbiInline`;
- `mfb_body_declared_errors.rs` covers `Body::Mfb`;
- `general_declared_errors.rs` covers the `general` members.

A temporary patch pointed the `Body::Mfb` sweep at each `Rewrite` target's helper
block. It was reverted, see bug-811's Resolution. It measured **19 of the 29**
`Rewrite` members out of step. For example:
- `crypto::hmac` declares no errors, but its body reaches `ErrOutOfMemory`,
  `ErrInvalidArgument`, `ErrOverflow`, `ErrIndexOutOfRange` and `ErrNotFound`.
- Every `compress` codec, and most of `crypto`, lack `ErrOutOfMemory`.

## Root Cause

`Body::Rewrite` members were outside the bug-698 and bug-807 sweeps. Their
descriptors were written by hand and never checked mechanically.

## Goal

- Every `Body::Rewrite` member declares exactly the errors its body can raise,
  with per-code reachability proof as in bug-698. A code proven unreachable goes in
  the guard's `(package, member, code)` exception table.
- The guard covers `Body::Rewrite`.
- Once it lands, `mfb audit`'s hand-written fallible lists
  (`src/audit/collect/source.rs`, kept as a floor by bug-811) can be deleted.

## Blast Radius

- `crypto`, `compress` and the other `Rewrite` members: their descriptors and man prose.
- Audit output, and the man/registry goldens for those members.

## Phases

- [x] Phase 1: add the guard for `Body::Rewrite` with a measured baseline; RED first.
      Commit: `31e5c8d46`
- [x] Phase 2: fill `errors` and align prose per member, with per-overload evidence.
      Commits: `f8a1f2d41` (compress), `adc0d8f8a` (randomInt body bug), `f5648d856` (crypto)
- [x] Phase 3: delete audit's hand lists. Commit: `2ed07b8f7`. The full suite and the
      golden regeneration are left to the batch gate (see Resolution).

## Resolution (2026-10-04)

**Guard (Phase 1, `31e5c8d46`).** `rewrite_sweep` resolves each `Body::Rewrite`
implementation's target `FUNC` exactly as the `Body::Mfb` sweep resolves a body (own
`FAIL`s, calls, `TRAP` modeled) and compares it with that implementation's own
`errors` — per overload, not the member union, because `crypto::hash`/`seal`/`open` mix
a `Rewrite` overload with a native one. A call back into the member counts only its
other implementations, so a declared code cannot justify itself. Two sound
narrowings were added to qualified-call propagation (both sweeps): a literal
`bits::sl`/`sr`/`sra` count in 0..63 raises no `ErrInvalidArgument`, and a member whose
implementations differ by first-parameter kind raises only what the inferred kind's
implementations declare (`collections::get` on a `List` cannot raise the `Map`
overload's `ErrNotFound`). The `Body::Mfb` sweep stayed green under both.

Measured, plain `is_empty` assertion: **RED, 24 of 34 implementations / 19 of 29
members** (`cargo test --release --bin mfb
every_body_rewrite_implementation_declares_exactly_the_errors_its_target_can_raise`),
committed as a ratchet over those 24 strings. Final: **0 of 34**, plain exact-match
assertion, no baseline (`cargo test --release --bin mfb declared_errors`: 25 passed).

**Per-code evidence (Phase 2).** Reachable codes were declared; every unreachable one
is proven per helper `FUNC` in the new `UNREACHABLE_QUALIFIED_CALL`
(`(FUNC, "pkg::member", code)`) and in `UNREACHABLE_GENERAL_CALL`, with the data-flow
proof beside each block (loop bounds over padded messages, fixed-size states/limbs/
tables, entry-point length checks, literal arguments). No code is excused because a
`TRAP` absorbs it.

| member (implementations) | added |
| --- | --- |
| `compress::inflate`, `zlibDecode`, `gzipDecode`, `deflate`, `zlibEncode`, `gzipEncode` | `ErrOutOfMemory` |
| `crypto::hmac` (2), `hkdf`, `pbkdf2`, `argon2id` (2), `shake256` (2), `exchange`, `decrypt` | `ErrOutOfMemory` |
| `crypto::convert` | `ErrInvalidArgument` (its own `FAIL`s), `ErrOutOfMemory` |
| `crypto::encrypt` (2) | `ErrOutOfMemory`, `ErrUnknown` (`crypto::randomBytes` for the ephemeral key) |

`crypto::hmac` and `crypto::hash` no longer call themselves total. `compress::crc32`,
`crypto::randomInt` (2), `uuid4`, `constantTimeEqual` and the seven `strings` members
already matched once the unreachable codes were proven. A 150 000-iteration bit-flip
fuzz of `compress::inflate` (plus 6 000 mutated zlib/gzip streams) raised only
`ErrInvalidFormat` or succeeded, agreeing with the decoder proofs.

**Body bug fixed (`adc0d8f8a`).** `crypto::randomInt(Integer, Integer)` computed `max -
min` and `span + 1` with the checked operators and leaked the undeclared
`ErrOverflow` (77050010) against its documented contract ("a span so large that `max -
min` overflows … is rejected with `ErrInvalidArgument`. Every other combination is
valid"): `randomInt(MIN, MAX)`, `(MIN, 0)`, `(-5, MAX)` raised 77050010 → now 77050002;
`randomInt(-1, MAX - 1)`, `(0, MAX)`, `(MIN, -1)` (span exactly `MAX`, 2^63 outcomes)
raised 77050010 → now return a value in range (one uniform 63-bit draw). RED/GREEN in
`tests/runtime/rt_crypto_random_int_span.rs`; the existing
`crypto-randomint-wide-range-rt` fixture prints the same seven lines.

**Audit (Phase 3, `2ed07b8f7`).** The whole-package rule and `is_fallible_builtin` are
deleted; a built-in is fallible exactly when it declares an error
(`every_builtin_is_fallible_exactly_when_it_declares_an_error`, RED on five members
the package rule over-reported: `io::isBuffered`/`isInputTerminal`/`isOutputTerminal`/
`isErrorTerminal`, `thread::isCancelled`). Nine hand-listed names were not members.
`mfb audit --format json` over all 1619 `tests/`/`examples/` projects, bug-staging-9
binary vs final: the 13 `.audit` fixtures are byte-identical; elsewhere only call rows
for those five total members disappear (four `tests/syntax/{io,threads}` functions flip
fallible → total) and `crypto.convert` rows appear. Three audit unit tests changed,
with the 4-question proof in the commit message.

**Verification.**

```
CARGO_TARGET_DIR=/tmp/b814-target cargo test --release --bin mfb declared_errors   # 25 passed
CARGO_TARGET_DIR=/tmp/b814-target cargo test --release --bin mfb audit::           # 90 passed
CARGO_TARGET_DIR=/tmp/b814-target cargo test --release --bin mfb spec              # 46 passed
MFB_TEST_EXE=/tmp/b814-target/release/mfb cargo test --release --test rt_crypto_random_int_span \
  --test cli_audit_audio_surface --test cli_audit_malformed_lock                  # 1 + 1 + 1 passed
MFB=/tmp/b814-target/release/mfb ./scripts/man-census.sh --memory-scope crypto compress  # unclassified 0
MFB=/tmp/b814-target/release/mfb ./scripts/man-census.sh --scope crypto compress         # internals 0
```

Rendered: `mfb man compress` (overview + six codec pages), `mfb man crypto` hmac, hash,
hkdf, pbkdf2, argon2id, shake256, convert, exchange, encrypt, decrypt, randomInt.

**Expected golden moves (not regenerated here).** `mfb man`/registry-dump goldens for
the 19 members above plus `crypto::hash` (prose); `.ir` of every fixture that imports
`crypto` (the `__crypto_randomInt` helper body grew, shifting later line numbers —
measured on `crypto-x448-valid`, `crypto-kdf-invalid`, `crypto-randomint-wide-range-rt`;
a `compress` fixture is unchanged) and any `.nir`/`.ncode`/byte-identity output of a
program calling `crypto::randomInt(Integer, Integer)`. No `.audit` golden moves.

**Left open.** `crypto::argon2id` accepts a `length` above 2^32 − 1, which RFC 9106 §3.1
forbids (its `le32` encodings truncate it); the page only says "at least 4", and such
a call would have to build a 4 GiB result, so it was not exercised here.
