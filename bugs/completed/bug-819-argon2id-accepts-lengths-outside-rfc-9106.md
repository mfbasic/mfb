# bug-819: `crypto::argon2id` accepts parameters outside RFC 9106

Last updated: 2026-10-04
Effort: small–medium (guess)
Severity: LOW
Class: Correctness (standards conformance)

Status: Closed (2026-10-04)
Regression Test: `src/codegen/builtins/crypto/helper_argon2id.rs` (`mod tests`,
three `body_enforces_rfc_9106_*` checks) plus
`tests/rt-behavior/crypto/crypto-argon2id-valid/src/main.mfb`'s `length-over-max`
case.

Found while fixing bug-814. Owner decision (2026-10-04): follow the RFC.

## Problem

`crypto::argon2id` accepts an output `length` above 2^32−1, which RFC 9106
§3.1 forbids (tag length T must be 4..2^32−1 bytes). The man page says only
"at least 4".

## Goal

Every argon2id input the API exposes is checked against RFC 9106 §3.1, and an
out-of-range value raises `ErrInvalidArgument` with a message naming the
parameter:
- tag length T: 4..2^32−1;
- parallelism p: 1..2^24−1;
- memory m (KiB): 8·p..2^32−1;
- passes t: 1..2^32−1;
- message (password) and nonce (salt) lengths: at most 2^32−1, and the nonce at
  least 8 bytes;
- secret and associated data, if exposed: at most 2^32−1.

Each bound is cited in the man page and the spec. The declared errors stay exact,
and the bug-698/807/814 guards must stay green. The output for valid parameters
does not change (RFC 9106 test vectors).

## Phases

- [x] RED tests, one per bound; the RFC test vectors as a control.
- [x] Add the checks; prose and spec; guards.

## Resolution (2026-10-04)

**What `crypto::argon2id` exposes, and what was already enforced.** The API
has no `secret`/associated-data parameters (RFC 9106 section 3.1's bounds on
those two do not apply here). Before this fix, `__crypto_argon2id`
(`src/codegen/builtins/crypto/helper_argon2id.rs`) already enforced
`parallelism` in `1..=16777215` (RFC's `1..=2^24-1`, exact), `iterations` in
`1..=4294967295` (RFC's passes `t`, `1..=2^32-1`, exact), `length ≥ 4` (RFC's
tag length lower bound, but with **no upper bound** — the bug), `len(salt) ≥
8` (stricter than RFC, which places no floor on the nonce, but again **no
upper bound**), and `memoryKiB` in `8×parallelism..=2097152`. That last one is
**stricter** than RFC's own `8×parallelism..=2^32-1` by design (the man page
already said so: "the largest memory RFC 9106 recommends" — kept unchanged,
since narrower-than-RFC is not the bug class here; nothing outside the RFC's
allowed set was ever accepted through it). `len(password)` had no bound at
all.

**The fix.** Added, in the same pre-allocation guard block,
before `len(blocks)`/`mem` is ever built: `length > 4294967295` (closing the
bug), `len(password) > 4294967295`, and `len(salt) > 4294967295`. All three
raise `ErrInvalidArgument` (`77050002`) naming the parameter, exactly like the
five checks already there. This also fixes a latent correctness issue, not
just a conformance gap: `__crypto_argon2H0` LE32-encodes `len(password)` and
`len(salt)` into the pre-hash input (RFC 9106 figure 1), and `__crypto_argon2Le32`
only ever emits the low 4 bytes — so a length at or above 2^32 would have
silently wrapped into the digest rather than raising.

**Tests.** The `length` bound is cheap to drive at full scale: it is a plain
`Integer` argument, so `attempt(32, 1, 1, 4294967296, 16)` in
`crypto-argon2id-valid/src/main.mfb` passes the boundary value directly — no
allocation happens, because the check runs before the output buffer is sized
(measured: `/usr/bin/time -l` on the built fixture reports ~83 MB peak RSS,
1.63s total for the whole file, not gigabytes). RED confirmed by code reading
(the unfixed body has no `length > …` check, so that call falls through to the
real derivation and attempts to size a ~4 GiB output buffer); GREEN confirmed
by running the fixed binary: `length-over-max=raised 77050002`.

The `password`/`salt` upper bound is NOT practically driveable at the real
2^32-1-byte boundary — building that much actual data, or letting it reach
`__crypto_argon2HPrime`'s output sizing, needs multiple GiB of real
allocation in the test process, which is exactly what RFC 9106 compliance is
supposed to make unreachable, not exercise. The regression test for these two
is a Rust-level check that the generated MFBASIC source text contains the
guard (`helper_argon2id.rs::tests::body_enforces_rfc_9106_{password,salt_upper}_bound`),
confirmed RED (`cargo test --release --bin mfb helper_argon2id::tests` failed
3/3 before the fix) then GREEN (3/3 after).

**Control.** `crypto-argon2id-valid`'s existing RFC 9106 §5.3 vector and every
other already-passing cost/parameter case are unaffected: rebuilding the
fixture and diffing its stdout against the committed `build.log` golden
(before adding the new test line) is byte-for-byte identical, including
`memory-4GiB=raised 77050002` (already covered — `attempt(4294967296, …)`
already exceeded the existing, stricter `memoryKiB` ceiling).

**Guards.** `cargo test --release --bin mfb declared_errors` — all 25 tests
pass, including
`declared_errors_all::every_crypto_member_declares_exactly_the_errors_its_lowering_raises`
and both `mfb_body_declared_errors` sweeps (bug-814's `Body::Rewrite` guard).
The new `FAIL error(77050002, …)` sites reuse the existing declared
`ErrInvalidArgument`, so no descriptor's `errors` list changed.

**Docs.** `man-census.sh --memory-scope crypto` and `--scope crypto` against
the rebuilt binary: 0 unclassified hits on both (new prose introduces no
banned memory- or compiler-internals vocabulary). `src/docs/spec/stdlib/10_crypto.md`'s
argon2id section now cites every bound against its RFC 9106 section 3.1
counterpart, including the two that are deliberately stricter than the RFC
(`salt`'s 8-byte floor, `memoryKiB`'s 2 GiB ceiling).

**Golden moves (not regenerated here; targeted tests only).**
`tests/rt-behavior/crypto/crypto-argon2id-valid/golden/{crypto-argon2id-valid.ast,crypto-argon2id-valid.ir,build.log}`
need a regen: one new `io::print` node/line for the `length-over-max` case.
`argon2id` is gated `HelperGate::WhenUsed`, and no other fixture under
`tests/` calls it (`grep -rl argon2id tests/` matches only this fixture), so
no other golden — including every `.ncodesum` byte-identity golden — is
affected. The companion-rendering concern from `.ai/testing-gates.md` ("a
registry `description` edit shifts every `.ir` golden's line numbers") was
checked and does NOT apply here: `func_argon2id.rs`'s `INTRO`/`DESC`/`EX` are
rendered only for `mfb man`, not into the synthesized MFBASIC companion;
rebuilding before and after the prose edit left the (pre-existing, unrelated)
`#CRYPTO_ED448_*` line numbers in `crypto-argon2id-valid.ir` at the identical
offset in both cases.

**Pre-existing, unrelated drift noted, not fixed here:** a clean rebuild from
this worktree's unmodified HEAD already disagrees with
`crypto-argon2id-valid`'s committed `.ir` golden around `#CRYPTO_ED448_*` line
numbers and a `crypto::randomInt` range-check shape — present before this
change, out of scope for bug-819, and not touched by this commit.
