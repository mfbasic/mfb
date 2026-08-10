# bug-428: two `.mir` goldens are stale on main (`%ret0`↔`%arg0` vreg rename)

Last updated: 2026-08-03
Effort: small (<1h)
Severity: LOW
Class: Test hygiene / stale golden

Status: FIXED (regenerated in plan-83 commit `a8e4bd1a9`; merged to main with plan-83)
Regression Test: the two `.mir` goldens below (regenerate + verify)

## Resolution

Fixed independently while landing **plan-83**: its `artifact-gate … all` run
surfaced the same 2 diffs, they were verified pre-existing at the base tip
(`171fc43cf`, a detached rebuild emits the identical `%arg0`), and the two `.mir`
goldens were regenerated in commit `a8e4bd1a9` — the diff is nothing but the
`%ret0`→`%arg0` rename (a pure x0 label change; `.ncode`/`.nobj`/`.run` all match),
exactly the fix prescribed below. No other goldens were touched. Confirmed by the
post-merge `artifact-gate … all` = 0 diffs.

The artifact gate (`bash scripts/artifact-gate.sh <exe> all`) reports exactly two
diffs on a clean `main` (tip `300b2a2f8` at time of writing), both in the
`macos-aarch64.mir` machine-IR dump:

- `tests/rt-behavior/control-flow/control-flow-if/golden/control_flow_if.macos-aarch64.mir`
- `tests/syntax/lexical/parser-hello-world/golden/parser_hello_world.macos-aarch64.mir`

Each diff is a **virtual-register rename only**: the golden says `%ret0`, the
current compiler deterministically emits `%arg0` for the same instruction (e.g.
`ldr_u64 dst offset 88`, `add_imm ... imm 9`). On aarch64 `%ret0` and `%arg0`
are both `x0`, so the emitted machine code is unchanged — the fixtures' execution
(`.run`) and every other golden PASS; only the `.mir` dump's vreg *label* drifted.

## Not introduced by bug-427

Found while running the artifact gate to land bug-427. Proven pre-existing:
regenerating both `.mir` with a **detached `300b2a2f8` release build** produces
the same `%arg0` (both differ from the committed golden there too). Both fixtures
import only `io` / are hello-world — they use no collections, resources, or
STATE, so the bug-427 change cannot touch their codegen. bug-427 adds zero new
golden diffs.

## Likely cause

A vreg-naming change (plausibly the recent plan-85 typed-ABI-token /
`RESULT_VALUE_REGISTER` work — see memory `x86-fixpoint-is-pure-rename`,
`remap-x86-abi-linear-vs-cfg`) shifted how a loaded value is labelled `%ret0`
vs `%arg0` in the `.mir` dump, and these two goldens were not regenerated in the
landing commit. This is the "no data object"/incomplete-regen shape.

## Failing Reproduction

```
scripts/artifact-gate.sh target/release/mfb all   # 2 diffs, both .mir above
```

or, per fixture:

```
mfb build -q -mir <copy of the fixture project>   # emits %arg0 where golden has %ret0
```

## Fix (once PROVEN correct)

Before regenerating, PROVE `%arg0` is the intended label (per AGENTS.md
"never edit a golden until proven wrong"): confirm the rename is the *only*
delta (it is — a pure `%ret0`→`%arg0` substitution on x0), the execution golden
already passes, and the current compiler emits it deterministically. Then
regenerate exactly these two `.mir` goldens and confirm the diff is nothing but
the rename. Do **not** mass-regenerate other goldens.
