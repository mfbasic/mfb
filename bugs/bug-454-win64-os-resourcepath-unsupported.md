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

> **Repro updated 2026-08-31.** The original command named
> `examples/tls-server`, which no longer exists — it was replaced by
> `examples/network-server` (`cb44b95b8`), and that example does not call
> `os::resourcePath`, so the old command now *passes* for the wrong reason.
> Use the self-contained project below instead.

```
mkdir -p /tmp/r454/src /tmp/r454/resources && : > /tmp/r454/resources/data.txt
cat > /tmp/r454/project.json <<'EOF'
{ "name": "r454", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [ { "root": "src", "role": "main", "include": ["**/*.mfb"] } ],
  "entry": "main", "targets": ["native"] }
EOF
cat > /tmp/r454/src/main.mfb <<'EOF'
IMPORT os
IMPORT io

FUNC main AS Integer
  io::print(os::resourcePath("data.txt"))
  RETURN 0
END FUNC
EOF
target/release/mfb build --target windows-x86_64 -q /tmp/r454
```

- Observed: `error: native backend does not support runtime call
  'os.resourcePath'`, exit 1.
- Expected: a windows-x86_64 executable, as produced for
  macos-aarch64 / linux-x86_64 / linux-aarch64 / linux-riscv64.

**Re-verified 2026-08-31** at `ba1c1750b` with a freshly built
`target/release/mfb`: the project above builds for `native` (macos-aarch64) and
`linux-x86_64` and fails for `windows-x86_64` with exactly the error above.

**Partially fixed since this was filed — scope is now narrower.** The sibling
call `os.executablePath`, which this document treats as sharing the acquisition,
**has** landed on Windows: it is in the win64 supported list
(`src/target/win_x86_64/mod.rs:55`) and has a lowering
(`src/target/win_x86_64/plan.rs:229`). Only `os.resourcePath` is missing —
`grep -n 'os.resourcePath' src/target/win_x86_64/mod.rs` returns nothing, while
`src/target/linux_common/mod.rs:98` and `src/target/macos_aarch64/mod.rs:83`
both list it. So the remaining work is to route `os.resourcePath` to the
exe-path acquisition that Windows already has, not to build that acquisition
from scratch as the References section implies.

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

- `mfb build --target windows-x86_64 /tmp/r454` (the Failing Reproduction
  project) succeeds, and
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

- [ ] Fixture cross-built to windows-x86_64 in a test
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
