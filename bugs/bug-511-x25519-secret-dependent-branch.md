# bug-511: X25519 Montgomery ladder conditionally swaps under a branch on the private-scalar bit (timing side-channel)

Last updated: 2026-09-05
Effort: small (<1h)
Severity: MEDIUM
Class: security (cryptographic timing side-channel)

Status: FIXED (branch pending). Found in audit-3, Surface 6 CRY-01; verified,
widened by one site the report missed, and fixed.

Regression Test: `src/codegen/builtins/crypto/mod.rs::curve25519_secret_paths_are_branch_free`
(RED before the fix), the structural twin of the existing
`curve448_secret_paths_are_branch_free`. Positive pin:
`tests/acceptance/src/crypto.mfb` TCASE
`"X25519 §5.2 raw scalar-multiplication vectors (bug-511 ladder pin)"`.

## Summary

The X25519 scalar multiplication swaps the ladder state inside `IF r = 1 THEN`,
where `r` is a bit of the private scalar. A secret-dependent branch leaks the
scalar bits through instruction-timing / cache behavior to a co-resident or
timing-capable attacker. The sibling implementations (X448, Ed25519) already use
constant-time masked selects; X25519 is the outlier. The scalar here is the
private key material used by `crypto::exchange` / reachable via `crypto::decrypt`.

## Mechanism

```mfbasic
# src/codegen/builtins/crypto/helper_x25519.rs:37-70
LET r AS Integer = bits::band(bits::sr(toInt(collections::get(z, byteIdx)), bitIdx), 1)
IF r = 1 THEN                 # <-- branch on a private-scalar bit
  LET ta AS List OF Integer = a
  a = b
  b = ta
  ...
END IF
... ladder step ...
IF r = 1 THEN                 # <-- and the unswap
  ...
END IF
```

The whole ladder step runs either the swap or not based on `r`, so both the branch
direction and the memory-access pattern depend on the secret. Masked constant-time
selection (conditional swap via `cswap(mask, a, b)` with `mask = 0 - r`) is the
standard fix and is what the neighboring curves use.

## Reproduction

Code-verified (the two `IF r = 1` blocks around the ladder step). A timing PoC
requires cycle-accurate measurement; the structural side-channel is direct from
the source.

## Verdict (2026-09-05) — the code was wrong, and the man page proves it

The report is correct and, on one point, understated. Both were established
before anything was changed.

**Which side is wrong is not a judgement call here.** `crypto::exchange`'s own
page already promises the property the code did not have
(`src/codegen/builtins/crypto/func_exchange.rs:45-50`, rendered by
`mfb man crypto exchange`):

> **Implementation.** Both ladders are portable MFBASIC software cores … a fixed
> 255-/448-iteration Montgomery ladder **whose conditional swap is branch-free**,
> so the output is byte-identical on every target and **no control flow depends
> on the private key**.

"Both ladders" was false for X25519. The only two ways to close that gap are to
make the code true or to make the page confess a timing side-channel in a
key-agreement primitive; the second is not a defensible product option, so there
is no product decision to refer upward. The spec agrees and was the more careful
of the two — `src/docs/spec/stdlib/10_crypto.md` attributed the branch-free
select swap to X448 *only*, so the spec never made the false claim; it simply
did not cover X25519. Both are now correct and explicit.

The repo also already enforces this property, for the other curve:
`src/codegen/builtins/crypto/mod.rs::curve448_secret_paths_are_branch_free`
asserts no `IF` in `__crypto_x448`, `__crypto_gf448Select`, the Ed448 ladder and
so on. X25519 had no such test. That asymmetry — not the absence of a
constant-time primitive — is why the branch survived: `__crypto_cswap128`,
`__crypto_ed448Cswap` and `__crypto_gf448Select` were all already in the package.

**The claim is in this repo's code, not in a vendored dependency.** `crypto` has
no third-party cryptography: every core is MFBASIC source assembled from
`src/codegen/builtins/crypto/helper_*.rs` (`func_exchange.rs:5-8` — "Pure-MFB
rewrite onto `__crypto_exchange` … no platform library, no `AbiFunction`"). The
fix is entirely in-tree.

### The report missed a second secret-dependent branch on the same path

`__crypto_pack25519` — which encodes the X25519 **shared secret** — chose its
canonical representative with

```
    IF b = 0 THEN
      t = m
    END IF
```

`b` is the borrow out of the trial subtraction of `p`, i.e. a function of the
value being packed. The X25519 result reaches this on every `crypto::exchange`,
so the branch leaked whether the shared secret needed reduction. The 448 twin
`__crypto_gf448Pack` already did this branch-free, with a
`__crypto_gf448Select` on the borrow, which is exactly how the reviewer of the
448 work saw it. Fixed in the same change; it is one line and the same primitive.

Audited and found already sound, so deliberately NOT changed:
`__crypto_isAllZero` (the RFC 7748 §6.1 low-order check) accumulates with
`bits::bor` over the whole list with no early exit; `__crypto_clampScalar` and
`__crypto_unpack25519` are branch-free; and `__crypto_edM`, `__crypto_car25519`
and `__crypto_inv25519` branch only on loop counters (`i`, `j`, `j2`, and
`inv25519`'s public exponent-bit position `a`) — the regression test pins that
distinction rather than banning `IF` outright.

## Best fix

Replace both `IF r = 1` swaps with a branchless masked conditional swap:
`mask = 0 - r`; for each limb, `t = mask & (a ^ b); a ^= t; b ^= t`. Mirror the
`__crypto`-level cswap helper the X448/Ed25519 paths already use, so all three
curves share one constant-time primitive.

## Non-goals

Do not change the computed result (X25519 output must stay identical for every
input); no MFBASIC surface change.

## Fix as landed (2026-09-05)

One new helper and three call sites; no MFBASIC surface change, no output change.

- **`src/codegen/builtins/crypto/helper_sel25519.rs` (new)** — `__crypto_sel25519(a, b, mask)`,
  the 16-limb branch-free select `a XOR (mask AND (a XOR b))`, i.e. TweetNaCl's
  `sel25519` and the 255-lane twin of `__crypto_gf448Select`. Kept separate from
  the 448 helper rather than shared, because the fields differ (16 × 16-bit limbs
  vs 16 × 28-bit) — the same split the package already makes between
  `__crypto_cswap128` and `__crypto_ed448Cswap`.
- **`helper_x25519.rs`** — both `IF r = 1` blocks become four selects each under
  `mask = 0 - r`, computed once per iteration. The old `a` is read into the new
  `b` before `a` is overwritten, so the swap is exact.
- **`helper_pack25519.rs`** — `IF b = 0 THEN t = m END IF` becomes
  `t = __crypto_sel25519(m, t, 0 - b)`.

### The non-goal ("must not change the computed result") is proven, not assumed

The same source built by the **pre-fix** binary and the **post-fix** binary prints
identical bytes for four X25519 vectors and one Ed25519 signature (Ed25519 also
routes through `pack25519`):

| vector | value |
| --- | --- |
| RFC 7748 §6.1 Alice→Bob and Bob→Alice | `4a5d9d5b…1742` |
| RFC 7748 §5.2 vector 1 | `c3da5537…8552` |
| RFC 7748 §5.2 vector 2 | `95cbde94…7957` |
| RFC 8032 §7.1 TEST 1 Ed25519 signature, empty message | `e5564300…100b` |

Cost: `0.16s` → `0.18s` user for that five-operation program (four selects per
ladder iteration now run unconditionally, where before roughly half the
iterations skipped the swap). That is the price of the property and is not
negotiable against it.

### Gates

- RED → GREEN: `cargo test --release --bin mfb curve25519_secret_paths_are_branch_free`
  — fails before the fix on `__crypto_x25519 must be branch-free`, passes after.
- Positive pin: the new acceptance TCASE with the RFC 7748 §5.2 vectors. Those
  vectors were **not** in the tree before (`grep -rn "c3da5537" tests/` was
  empty) even though `helper_x25519.rs` and `func_exchange.rs` both cite them.
- `scripts/artifact-gate.sh target/release/mfb all` — 1387 tests, 1924 goldens,
  **0 diffs**.
- `scripts/test-accept.sh` — **1409 ran, 0 mismatches**. Before the golden sync
  it reported 18 mismatches, and every one was a `.ir` golden of a
  crypto-importing fixture. Classified line by line: the only semantic deltas
  tree-wide are the three removed `IF` blocks, the added `#crypto_sel25519`
  function, and its call sites. Everything else is injected-source renumbering
  (the crypto package grew 7 lines). No `.run`, no `build.log`, no `.ast` golden
  moved — that is the proof that no program's behaviour changed.
- `.ncodesum`: 9 refreshed (5 targets of `byte-identity/crypto`, 4 of
  `crypto-ec-valid`) via `bash scripts/regen-ncodesum.sh`; 143 goldens
  considered, nothing outside `crypto` moved.
- `scripts/man-run-examples.sh crypto --run` — 29 examples, 29 built, 29 ran, 0 failed.

### Doc sync

- `func_exchange.rs`'s "Both ladders … branch-free … no control flow depends on
  the private key" is **left exactly as written** — it is now true. It was the
  evidence that the code was wrong, so changing it would have been the wrong
  half of the fix.
- `src/docs/spec/stdlib/10_crypto.md` key-agreement bullet rewritten: it named
  the branch-free select swap for X448 only. It now states the property for both
  curves, names both primitives, and records that the packers' conditional
  reduction and the all-zero low-order check are branch-free too.
- Module docs on `helper_x25519.rs` and `helper_pack25519.rs` corrected — the
  first previously *documented* the branch ("a plain branch on the key bit").

## Prior art

audit-2 CRY-02/CRY-03 (constant-time compare, `S ≥ L`) are confirmed fixed; the
ladder-swap timing gap is new (searched `cswap`, `constant time`, `ladder`,
`x25519`, `branch`). Related lower-severity items from the same pass: CRY-02
(AES S-box/GHASH table-driven, non-constant-time — LOW), CRY-08 (Poly1305 final
conditional subtraction branches — NTH).
