# bug-199: macOS thread runtime-imports omit transferResource/acceptResource → unresolved pthread symbols

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: correctness (platform: macos-aarch64)

Status: Open
Regression Test: tests/ (macOS program using thread resource transfer without thread.start links)

The macOS `runtime_imports` thread match arm omits
`thread.transferResource`/`thread.acceptResource`, so those helpers declare no
`pthread_mutex_*`/`pthread_cond_*` imports. This is the exact defect bug-176 C
fixed on all three Linux targets but never applied to macOS.

## Failing Reproduction

A macOS program whose reachable helper set includes
`thread.transferResource`/`acceptResource` but **not** `thread.start` (whose arm
otherwise donates the pthread symbols). Observed: the resource-plane helper
references `_pthread_mutex_lock`/`_pthread_cond_*` with no matching
`PlatformImport` → unresolved dynamic symbol at load. Masked whenever
`thread.start` is co-emitted (the usual case), so latent. Expected: the pthread
imports are declared for the resource-transfer helpers too.

## Root Cause

`src/target/macos_aarch64/plan.rs:572-574` — the thread arm lists
`thread.start`/`send`/`receive` but not `thread.transferResource`/`acceptResource`;
the Linux targets list them (`src/target/linux_x86_64/plan.rs:276-279`).

## Non-goals

- Do not change Linux plans (already correct).
- Do not add imports for the never-lowered `thread.drop`/`read`/`emit` entries
  (optionally drop them).

## Blast Radius

- macOS thread import arm only.

## Fix Design

Add `"thread.transferResource" | "thread.acceptResource"` to the macOS thread
pthread-import arm, mirroring `linux_x86_64/plan.rs:276-279`.
