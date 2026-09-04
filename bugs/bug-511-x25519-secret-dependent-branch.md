# bug-511: X25519 Montgomery ladder conditionally swaps under a branch on the private-scalar bit (timing side-channel)

Last updated: 2026-09-03
Effort: small (<1h)
Severity: MEDIUM
Class: security (cryptographic timing side-channel)

Status: Open (found in audit-3, Surface 6 CRY-01; code-verified by the lead)

Regression Test: not directly timing-testable in CI; assert the swap is a masked select (no `IF` on a scalar bit) by inspecting the emitted/source form.

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

## Best fix

Replace both `IF r = 1` swaps with a branchless masked conditional swap:
`mask = 0 - r`; for each limb, `t = mask & (a ^ b); a ^= t; b ^= t`. Mirror the
`__crypto`-level cswap helper the X448/Ed25519 paths already use, so all three
curves share one constant-time primitive.

## Non-goals

Do not change the computed result (X25519 output must stay identical for every
input); no MFBASIC surface change.

## Prior art

audit-2 CRY-02/CRY-03 (constant-time compare, `S ≥ L`) are confirmed fixed; the
ladder-swap timing gap is new (searched `cswap`, `constant time`, `ladder`,
`x25519`, `branch`). Related lower-severity items from the same pass: CRY-02
(AES S-box/GHASH table-driven, non-constant-time — LOW), CRY-08 (Poly1305 final
conditional subtraction branches — NTH).
