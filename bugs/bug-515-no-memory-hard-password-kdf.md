# bug-515: PBKDF2 is the only password KDF, and its own man page tells you not to use it

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: `tests/` — new `rt_crypto_argon2id` vectors (Phase 2)

`crypto::pbkdf2` is the only password-based key-derivation function the `crypto`
package offers. Its own description then says:

> For **new** password storage rather than the RFC 8018 / WPA2 legacy profile
> … prefer a memory-hard function (Argon2id, scrypt, or bcrypt) where one is
> available.

None is available. `grep -rniE "argon2|scrypt|bcrypt" src/codegen/builtins/`
returns exactly one non-Windows-CNG hit: that advisory sentence itself. A
developer following the package's own guidance has nowhere to go, so the
realistic outcome is that they use PBKDF2 anyway — for the one job the page says
not to use it for — or roll their own.

The single correct behavior a fix produces: `crypto` offers a memory-hard
password hash (Argon2id) that passes the RFC 9106 test vectors, and the PBKDF2
advisory points at it by name instead of at "where one is available".

References:

- `src/codegen/builtins/crypto/func_pbkdf2.rs:37` — the advisory with no target
- `mfb man crypto pbkdf2`
- RFC 9106 (Argon2), §5 test vectors
- Spike: none needed — the gap is an absence, established by the grep below

## Failing Reproduction

The finding is an absence, so the reproduction is the census:

```
grep -rniE "argon2|scrypt|bcrypt" src/codegen/builtins/ | grep -v BCryptGenRandom | grep -v bcrypt_call
```

- Observed: one line —
  `crypto/func_pbkdf2.rs:37: passwords, prefer a memory-hard function (Argon2id, scrypt, or bcrypt) where one is`
  (every other `bcrypt` hit is the Windows CNG `bcrypt.dll` RNG/signing seam,
  unrelated to password hashing.)
- Expected: a `crypto::argon2id` registry function alongside `crypto::pbkdf2`.

Contrast case: the package is *not* short of the primitives Argon2id needs. It
already ships BLAKE2b's sibling machinery in software — `crypto::shake256`
(`func_shake256.rs`), a full `bits` layer, and `crypto::hkdf`/`crypto::hmac`
cores — all "pure MFBASIC software cores computed over the `bits` package". So
this is a missing member, not a missing capability.

## Root Cause

Not a defect in existing code — a scope gap. `src/codegen/builtins/crypto/`
registers exactly one password KDF (`func_pbkdf2::register`), and the package's
`mod.rs` function list has no memory-hard entry. The advisory text was written
against a general best-practice recommendation rather than against this
package's own surface, so it names three functions the package does not have.

## Goal

- `crypto::argon2id(password, salt, timeCost, memoryKiB, parallelism, length)`
  exists, is a pure software core like the rest of the package, and produces
  byte-identical output on every target.
- It passes the RFC 9106 §5 Argon2id test vector.
- `func_pbkdf2.rs`'s advisory names `crypto::argon2id` instead of "where one is
  available".

### Non-goals (must NOT change)

- `crypto::pbkdf2` itself. It is the correct and required answer for RFC 8018,
  WPA2, and any peer that specifies PBKDF2; it must keep its exact behavior,
  signature and output.
- Adding a third-party crypto dependency. Every algorithm in this package is
  reproduced clean-room over `bits`; Argon2id must be too.
- Adding scrypt and bcrypt as well. One good memory-hard option is the goal;
  three is scope creep. If a peer requires scrypt specifically, that is a
  separate bug.
- **Tempting wrong fix, forbidden:** softening the advisory in `func_pbkdf2.rs`
  so it stops pointing at functions that do not exist. That resolves the
  *contradiction* by removing the correct advice, and leaves users with no
  memory-hard option at all.

## Blast Radius

- `src/codegen/builtins/crypto/func_pbkdf2.rs` — advisory text updated by this bug.
- `src/codegen/builtins/crypto/mod.rs` — new `func_argon2id::register` row.
- `src/codegen/builtins/crypto/helper_*.rs` — Argon2id needs BLAKE2b; check
  whether the existing SHA-512/SHAKE256 helpers share any reusable
  little-endian word machinery before writing new ones.
- `repository/` — unaffected; it does not hash passwords through this package
  (verify with `grep -rn "pbkdf2" repository/` in Phase 1).
- `tls`/`http` — unaffected; neither derives keys from passwords.

## Fix Design

Add `crypto::argon2id` as a `Body::mfb` software core over `bits`, matching how
`crypto::shake256` and the Ed25519/X448 cores are built. Argon2id needs
BLAKE2b-512 and the variable-length hash `H'`, neither of which the package has
today, so the work splits:

1. BLAKE2b-512 core + its own test vectors (RFC 7693 §B).
2. Argon2's `H'` variable-length hash and the compression function `G`.
3. The Argon2id indexing/filling passes, memory as a `List OF Byte` block.
4. The registry member, its parameter validation, and the man page.

The correctness risk is concentrated in step 3 — Argon2's data-dependent and
data-independent addressing differ per pass and per slice, and getting the
segment/lane arithmetic wrong produces a *plausible-looking* digest that fails
only against the official vector. Pin against RFC 9106 §5 before anything else,
per the project's "write the reference first" rule.

Rejected: shelling out to a platform library (`libargon2`, CNG). The package's
stated contract is byte-identical output on every target with no platform
crypto library; a platform-backed member would break that and is not available
on all three hosts anyway.

Rejected: exposing Argon2id only as a `repository`-internal helper. The
advisory is in the *public* man page, so the answer has to be public.

## Phases

### Phase 1 — vectors + audit (no behavior change)

- [ ] Pull the RFC 9106 §5 Argon2id vector and RFC 7693 §B BLAKE2b vectors as
      committed fixtures. Do not hand-transcribe them from prose.
- [ ] Write the Rust reference implementation first and pin it against those
      vectors, per the project's hand-written-core rule.
- [ ] `grep -rn "pbkdf2" repository/ src/` and record whether anything in-tree
      derives a key from a password today.

Acceptance: the Rust reference reproduces the official vectors byte-for-byte;
the in-tree consumer audit is written into Blast Radius above.
Commit: —

### Phase 2 — the fix

- [ ] BLAKE2b-512 core over `bits`, gated by its own vector test.
- [ ] Argon2id core, gated by the RFC 9106 vector.
- [ ] Register `crypto::argon2id`; write its man page per `.ai/man-content.md`.
- [ ] Point `func_pbkdf2.rs`'s advisory at `crypto::argon2id`.

Acceptance: the vector tests pass; `mfb man crypto argon2id` renders; the
pbkdf2 page no longer recommends a function that does not exist.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate the `.ncodesum` goldens the new package member shifts (run the
      regen scripts under **bash**, not zsh).
- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh crypto --run`; `scripts/man-census.sh --fill crypto`.
- [ ] Confirm byte-identical `argon2id` output on macOS, Linux and Windows.

Acceptance: full suite green; the same digest on all three hosts.
Commit: —

## Validation Plan

- Regression test: RFC 9106 §5 Argon2id vector and RFC 7693 §B BLAKE2b vectors,
  as committed fixtures.
- Runtime proof: the same password/salt/cost parameters producing the same
  digest on all three platforms.
- Doc sync: new `crypto::argon2id` page; `func_pbkdf2.rs` advisory; the crypto
  package intro's function list; `src/docs/spec/**` if it enumerates crypto members.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Parameter shape: RFC 9106's `(t, m, p)` spelled out as
  `timeCost`/`memoryKiB`/`parallelism`, vs. a single `crypto::Argon2Profile`
  enum with vetted presets. **Recommend the explicit parameters**, matching
  `crypto::pbkdf2`'s explicit `iterations`, with the man page carrying the
  recommended values — an enum freezes today's hardware assumptions into the
  language.

## Summary

The risk is the Argon2id core itself: it is the largest software primitive the
package would gain, and a subtly wrong addressing pass yields a digest that
looks fine and matches nothing. Pinning the RFC 9106 vector before writing any
MFBASIC is what makes that risk bounded. `crypto::pbkdf2` and every existing
primitive are untouched.
