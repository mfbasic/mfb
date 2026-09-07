# bug-515: PBKDF2 is the only password KDF, and its own man page tells you not to use it

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Security

Status: Fixed
Regression Test: `tests/rt-behavior/crypto/crypto-argon2id-valid`

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


## USER DECISION (2026-09-06) — both spellings, and the profile is a wrapper

Ruling: ship **both** an explicit-cost overload and a profile overload, with one
structural constraint the user stated directly — **the profile overload calls
the full version underneath.** It is not a second implementation and not a
second parameter table; it resolves its profile to concrete
`memoryKiB`/`iterations`/`parallelism` and calls the explicit member.

That matters for two reasons beyond tidiness:

- There is exactly ONE place the cost parameters are validated and one place
  they are used, so the two spellings cannot drift into disagreeing about what
  `Moderate` means.
- The profile constants become the only thing that is retunable later, and
  retuning them cannot change the explicit member's behavior.

The explicit member exists because a caller VERIFYING a hash produced elsewhere
must be able to name a specific published parameter set; the profile spelling is
what the man page recommends for producing one.

Still open, and answerable from the implementation rather than by the user: what
an over-large `memoryKiB` does. It must be a raised error, not an allocation
attempt — decide the error and pin it.

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

- [x] Pull the RFC 9106 §5 Argon2id vector and RFC 7693 §B BLAKE2b vectors as
      committed fixtures. Do not hand-transcribe them from prose.
- [x] Write the Rust reference implementation first and pin it against those
      vectors, per the project's hand-written-core rule.
- [x] `grep -rn "pbkdf2" repository/ src/` and record whether anything in-tree
      derives a key from a password today.

Acceptance: the Rust reference reproduces the official vectors byte-for-byte;
the in-tree consumer audit is written into Blast Radius above.
Commit: —

### Phase 2 — the fix

- [x] BLAKE2b-512 core over `bits`, gated by its own vector test.
- [x] Argon2id core, gated by the RFC 9106 vector.
- [x] Register `crypto::argon2id`; write its man page per `.ai/man-content.md`.
- [x] Point `func_pbkdf2.rs`'s advisory at `crypto::argon2id`.

Acceptance: the vector tests pass; `mfb man crypto argon2id` renders; the
pbkdf2 page no longer recommends a function that does not exist.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] Regenerate the `.ncodesum` goldens the new package member shifts (run the
      regen scripts under **bash**, not zsh).
- [x] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [x] `scripts/man-run-examples.sh crypto --run`; `scripts/man-census.sh --fill crypto`.
- [x] Confirm byte-identical `argon2id` output on macOS, Linux and Windows
      (by construction: a pure MFB core over `bits`, and the five `.ncodesum`
      cross-target goldens are regenerated and gated).

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


## Resolution (2026-09-06)

`crypto::argon2id` ships in both spellings, over one body.

### What was built

- `__crypto_blake2b*` — unkeyed BLAKE2b at any output length 1..64 (RFC 7693),
  reusing the existing `__CRYPTO_IV512` global (BLAKE2b's IV *is* SHA-512's, so
  the package gained no second copy of it).
- `__crypto_argon2*` — `H'`, `H_0`, the permutation `P`, the compression
  function `G`, the reference-index mapping, and the pass/slice/lane fill.
- `__crypto_argon2id(password, salt, memoryKiB, iterations, parallelism, length)`
  — the ONE validation site and the ONE fill site.
- `__crypto_argon2idProfile(password, salt, profile, length)` — resolves
  `crypto::Argon2Profile` to three constants and calls the body above. That is
  its entire content.

All 15 helper chunks are `HelperGate::WhenUsed(["argon2id"])`, so a program that
imports `crypto` without calling `crypto::argon2id` carries none of the ~390
lines of new source. That is why the golden churn below is only five lines wide.

### Decisions the implementation answered

- **Over-large `memoryKiB` raises, and the ceiling is 2097152 KiB (2 GiB)** —
  RFC 9106 §4's largest recommended memory. Above it, and below
  `8 × parallelism`, the call raises `ErrInvalidArgument` (`77050002`, the code
  `crypto::pbkdf2` already uses for the same class of fault) *before* any memory
  is taken. No new `Err*`, so no `data_objects.rs` row and no
  `standard_error_messages()` churn. `salt` must be ≥ 8 bytes and `length` ≥ 4,
  both RFC 9106 §3.1 minima; `parallelism` is 1..16777215.
- **No `secret` / `associatedData` parameters.** RFC 9106 §5.3's published vector
  carries both, so it cannot be run through this member as shipped. Rather than
  invent surface to make one vector reachable, the vectors in the regression
  fixture are the no-secret, no-associated-data case at the RFC's own cost
  parameters, cross-checked against two independent implementations (below).
- **Two profiles, not three.** Both constants sets are citable:
  `Minimum` = OWASP's minimum configuration `(19456, 2, 1)`; `Recommended` =
  RFC 9106 §4's SECOND RECOMMENDED option `(65536, 3, 4)`. A third, higher
  profile was prototyped at RFC 9106's FIRST RECOMMENDED `(2097152, 1, 4)`: it
  is correct (it matches OpenSSL) but takes 83 s and 15.6 GiB of resident memory
  here, which is not something to put behind a friendly name. The explicit
  overload still reaches it.

### Oracles

The Rust reference written first (`/tmp/argon2ref`, not committed) reproduces
**RFC 9106 §5.3**'s pre-hashing digest and tag byte-for-byte
(`0d640df58d78766c…e659`) and **RFC 7693 Appendix A**'s
`BLAKE2b-512("abc")`. Every digest the MFBASIC core produces was then checked
against two further implementations that agree with it exactly: **OpenSSL
3.6.2**'s `ARGON2ID` KDF and the **RustCrypto `argon2` crate, pinned `=0.5.3`**.
The MFBASIC core matched all seven development vectors on its first run.

### Cost

A portable MFBASIC core is far slower than a C Argon2: about 22 MiB of Argon2
memory per second on this host (macOS/aarch64, release, `-O2`), so `Minimum`
takes ~1.4 s and `Recommended` ~7 s. Two rounds of tuning got there from ~4.5x
slower: fusing `a + b + 2·trunc(a)·trunc(b)` into one limb-arithmetic helper
instead of nested `__crypto_add64` calls, writing the permutation over locals
rather than a list per mixing step, replacing every `collections::append` growth
loop with a preallocated `set` loop, and giving the compression function offsets
into the block matrix instead of three sliced blocks.

One MFBASIC runtime property is worth recording because it bounds what this
member can offer: **transient collection values are not reclaimed until the
function that created them returns**, so a call that performs millions of block
fills without returning accumulates roughly 3 KiB per fill. Measured: 655 MB
resident for a 64 MiB / 3-iteration derivation, 15.6 GiB for 2 GiB / 1
iteration. The man page states the resulting rule of thumb. This is not specific
to Argon2 — any long-running MFBASIC loop that builds temporaries has it.

### Cross-platform runtime proof

The regression fixture was cross-compiled and executed on every reachable target;
all five runs printed byte-identical output (md5 `b66134d365bad50ccef0bc93c6da1b0d`
of the whole program output, checked against the macOS/aarch64 run):

| target | box | wall clock |
|---|---|---|
| macos-aarch64 | host | 3.3 s |
| linux-x86_64 glibc | 2228 | 46 s (1 core) |
| linux-x86_64 musl | 2227 | — |
| linux-riscv64 musl | 2229 | 44 s |
| windows-x86_64 | 2230 | — |

### Golden delta

Every existing golden that moved, moved by **five lines**, and nothing else:
the `crypto::Argon2Profile` enum renders into `builtins/crypto.mfb` ahead of the
helper and member bodies, so line numbers below it (and the line numbers baked
into `ErrorLoc` constructors) shift by 5, and the `.ir` `types` array gains one
`crypto.Argon2Profile` entry. Affected: 16 `crypto` `.ir` goldens plus
`syntax/security/bug96_audit_tls_http_crypto`, and 9 `.ncodesum` goldens
(`byte-identity/crypto` × 5 targets, `crypto-ec-valid` × 4). No fixture outside
`crypto` moved — `scripts/regen-ncodesum.sh` refreshed 144 goldens and only
those 9 differed.
