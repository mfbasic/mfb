# bug-454: `os.resourcePath` (and its exe-path acquisition) is unimplemented on windows-x86_64 — valid cross-builds are rejected

Last updated: 2026-08-25
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (portable API missing on one target)

Status: Open
Regression Test: — (Phase 1 adds a windows-x86_64 cross-build of a resourcePath fixture; runtime proof needs a Windows host or the existing PE test rig)

A project using `os::resourcePath` builds for macOS and Linux but is rejected
when cross-compiled to `windows-x86_64`: the capabilities gate reports the
runtime call as unsupported, so `examples/tls-server` (and any user of the
API) cannot target Windows at all. plan-55-B implemented the call on
macOS (`src/target/macos_aarch64/plan.rs:266`, exe-path acquisition) and Linux
(`src/target/linux_common/plan.rs:223`, `readlink("/proc/self/exe")`), and the
Win64 twin was never added. **The single correct behavior a fix produces:
`os.resourcePath` (and `os.executablePath`, which shares the acquisition)
lowers on windows-x86_64 via `GetModuleFileNameW`, and the cross-build
succeeds with the same semantics as the other targets.**

References:

- plan-55-B (the `os.executablePath`/`os.resourcePath` design and its per-OS
  acquisition table).
- Memory note `adding-a-call-to-an-existing-native-pkg.md` — the per-target
  `SUPPORTED_RUNTIME_CALLS` gate this trips.
- Found during the optimizer worktree's all-targets examples verification
  (2026-08-24).

## Failing Reproduction

```
target/release/mfb build --target windows-x86_64 -q examples/tls-server
```

- Observed: `error: native backend does not support runtime call
  'os.resourcePath'`, exit 1.
- Expected: a windows-x86_64 executable, as produced for
  macos-aarch64 / linux-x86_64 / linux-aarch64 / linux-riscv64.

| Environment | Details | Result |
| --- | --- | --- |
| windows-x86_64 | any project calling `os::resourcePath` | fails ✗ |
| macos-aarch64 | same source (plan-55-B lowering) | works ✓ |
| linux-* | same source (`/proc/self/exe` lowering) | works ✓ |

## Root Cause

The Win64 target never received plan-55-B: `src/target/win_x86_64` has no arm
for `os.resourcePath`/exe-path acquisition in its plan lowering, so the call
never enters its supported-runtime-calls set and
`src/target/shared/validate/capabilities.rs` rejects the build up front (the
gate is doing its job; the lowering is what is missing). The macOS and Linux
implementations live at `src/target/macos_aarch64/plan.rs:266` and
`src/target/linux_common/plan.rs:223` respectively; the Windows equivalent of
their acquisition step is `GetModuleFileNameW` (kernel32), with the
resource-directory derivation shared with the other targets.

## Goal

- `mfb build --target windows-x86_64 examples/tls-server` succeeds, and
  `os::resourcePath` on a Windows host returns the executable-adjacent
  resource path with the same joining semantics as macOS/Linux.

### Non-goals (must NOT change)

- macOS/Linux lowerings and their goldens — byte-untouched.
- The capabilities gate itself — it must keep rejecting genuinely-unsupported
  calls; do NOT "fix" this by whitelisting the call without a lowering.
- No UTF-16 shortcuts: `GetModuleFileNameW` returns UTF-16 — conversion must
  go through the runtime's existing UTF-16→UTF-8 path (see the Win console
  handling in `.ai/arch-abi.md`), not a lossy byte cast.

## Blast Radius

- `src/target/win_x86_64` plan lowering — fixed by this bug
  (`os.resourcePath` + `os.executablePath` twin arm).
- Other `os.*` calls on Win64 — audit in Phase 1: diff the macOS/Linux
  supported sets against Win64's and list any additional missing calls; each
  becomes either in-scope here (same acquisition) or its own bug.
- The capabilities gate (`validate/capabilities.rs`) — unaffected (data-driven
  by the per-target sets).

## Fix Design

Add the plan-55-B arm to the Win64 plan lowering: acquire the module path via
`GetModuleFileNameW` (growing buffer loop per the API contract), convert
UTF-16→UTF-8 through the runtime's existing helper, then reuse the shared
resource-path derivation. Register the call(s) in the target's supported set.
Risk concentrates in the wide-string conversion and long-path (`\\?\`)
handling; PE import bookkeeping for kernel32 already exists.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Fixture (or reuse tls-server) cross-built to windows-x86_64 in a test
      asserting today's rejection message.
- [ ] Audit: diff Win64's supported `os.*` set against macOS/Linux; verdict
      per missing call.

Acceptance: test fails with the documented error; audit table filled in.
Commit: —

### Phase 2 — the fix

- [ ] Win64 plan arm for `os.executablePath`/`os.resourcePath`
      (`GetModuleFileNameW` + UTF-16→UTF-8 + shared derivation); add to the
      supported set.

Acceptance: the cross-build succeeds; the Phase 1 test flips to asserting an
artifact.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `artifact-gate.sh all` — the new lowering appears only in
      windows-x86_64 plans of resourcePath users; classify/regen exactly
      those (`regen-ncodesum.sh` scope caveat from memory:
      `win64-change-ripples-all-io-importers` — expect the windows `.ncodesum`
      ripple and re-sync ALL io-importers, not just one).
- [ ] `cargo test --no-fail-fast`; full `test-accept.sh`.
- [ ] Runtime proof on a Windows host (`.ai/remote_systems.md`) — print
      `os::resourcePath()` and verify the exe-adjacent path.

Acceptance: suite green; golden delta is exactly the windows-x86_64
resourcePath users; Windows-host run correct.
Commit: —

## Validation Plan

- Regression test: the Phase 1 cross-build test (fails today → asserts
  success + artifact after).
- Runtime proof: Windows-host execution printing the path.
- Doc sync: `mfb man os resourcePath` platform notes if they enumerate
  targets; `.ai/arch-abi.md` Windows section.
- Full suite: `cargo test --no-fail-fast`, `artifact-gate.sh all`,
  `test-accept.sh`.

## Open Decisions

- Buffer strategy for `GetModuleFileNameW` (fixed MAX_PATH vs. grow-on-
  truncation). Recommended: grow-on-truncation loop — long paths are real on
  modern Windows.

## Summary

A contained per-target feature gap: the risk is UTF-16 conversion and the
windows `.ncodesum` ripple (known from memory to hit every io-importing
fixture), not the design — macOS/Linux define the semantics to copy.
