# bug-22: `mfb audit` never discloses network/process/environment/clock/randomness capabilities or native LINK symbols — false-negative capability report

Last updated: 2026-07-08
Effort: medium (1h–2h)

`mfb audit` is meant to answer "what host capabilities does this project use?"
Two collection gaps make it silently under-report:

**(a) `builtin_capability` maps only fs/io/thread (MEDIUM, security false-negative).**
`src/audit/collect/source.rs::builtin_capability` (`:455-462`) returns a capability
only for `fs`→filesystem, `io`→terminal, `thread`→threads; every other package
returns `None`. Since the sole producer of a `PermissionEntry` is the `visit`
closure in `collect_source` (`source.rs:~24-33`), gated on `builtin_capability`, no
entry is ever created for `net`, `os`, `math`(rand), or `datetime`. The
`permission_findings` arms for `"network"`/`"process"`/`"environment"`/`"clock"`/
`"randomness"`/`"native"` (`findings.rs:196-203`, codes AUDIT-PERM-NETWORK / -PROCESS
/ -ENVIRONMENT / -CLOCK / -RANDOMNESS / -NATIVE) are therefore **dead** — those
codes can never be emitted. A user asking `mfb audit` "does this package touch the
network / read env / spawn processes?" gets a false "no."

**(b) `native_links` is hard-coded empty (LOW, dead report path).**
The sole production constructor of `AuditReport`, `collect()` (`collect/mod.rs:60-72`),
sets `native_links: Vec::new()` (`:67`) and nothing ever populates it. So the
`NativeLinkEntry` struct, the JSON `nativeLinks` renderer (`json.rs:283-298`), and
the text "Native links:" block (`text.rs:110-120`) are unreachable in production —
a project's native `LINK` **symbols** are never reported. (Native LINK *resource
types* are reported via `collect_native_resources`, so this looks like a
superseded-but-not-removed surface.)

The single correct behavior a fix produces: `mfb audit` discloses every host
capability a project actually uses (network, process, environment, clock,
randomness, native), so the AUDIT-PERM-* codes and the native-links section are
either reachable or removed.

Severity MEDIUM (driven by the security false-negative in (a); (b) is LOW
dead-code).

References:

- `src/audit/collect/source.rs:455-462` (`builtin_capability`, only fs/io/thread),
  `:~24-33` (the `visit`/`PermissionEntry` producer gated on it).
- `src/audit/collect/findings.rs:194-205` (`permission_findings` — the
  network/process/environment/clock/randomness/native arms are dead).
- `src/audit/collect/mod.rs:60-72` (sole production `AuditReport` ctor;
  `native_links: Vec::new()` at `:67`).
- `src/audit/report.rs:31,313,421` (`native_links` field + test-only populated
  ctors), `src/audit/json.rs:283-298`, `src/audit/text.rs:110-120` (dead renderers).
- Contrast: `is_fallible_call` (`source.rs:465`) DOES recognize `net`/`json`, so
  those show up as fallible call sites (capability `None`) — confirming the gap is
  in `builtin_capability`, not the walker.
- Found during goal-01 review of `src/audit/**`.

## Failing Reproduction

A project that calls `net::connectTcp(addr)`, `os::getEnv("X")`, `math::rand()`,
`datetime::now()`, and a native `LINK` function; run `mfb audit`.

- Observed: the Permissions section lists nothing for network/process/environment/
  clock/randomness; no AUDIT-PERM-NETWORK/-PROCESS/-… finding; no "Native links:"
  section. `nativeLinks` is `[]`.
- Expected: each used capability is disclosed with its AUDIT-PERM-* finding, and
  native LINK symbols appear in the native-links section.

Contrast: fs/io/thread capabilities ARE disclosed correctly today.

## Root Cause

Capability collection is incomplete: `builtin_capability` maps three packages, and
`native_links` is never populated. The downstream finding codes and renderers were
built for the full capability set but the collection layer never feeds them.

## Goal

- `mfb audit` emits a permission finding for every host capability a project uses
  (filesystem, terminal, threads, network, process, environment, clock, randomness,
  native), and reports native LINK symbols (or the native-links surface is deleted
  if `native_resources` supersedes it).

### Non-goals (must NOT change)

- fs/io/thread capability reporting (correct today).
- The finding-code identifiers (they already exist).

## Blast Radius

- `builtin_capability` (`source.rs:455`) — extend the mapping. The `os` package
  splits into process vs environment per builtin; `math::rand`/`seed`→randomness;
  `datetime`→clock; `net`→network.
- `native_links` population (`collect/mod.rs`) — feed from the AST LINK blocks, or
  remove the dead struct/renderers.

## Fix Design

Extend `builtin_capability` (or add a parallel capability resolver keyed on the
specific builtin for `os`, which mixes process and environment operations) so
net/os/math/datetime/native produce `PermissionEntry`s. Populate `native_links`
from the project's `LINK` declarations in `collect()`, or delete
`NativeLinkEntry`/`native_links`/its renderers if `native_resources` is the
intended replacement (decide explicitly). Add audit acceptance fixtures covering
each newly-disclosed capability.

## Phases

### Phase 1 — failing test + audit

- [ ] Add `mfb audit` fixtures for a project using net/os/math::rand/datetime and a
      native LINK; assert the AUDIT-PERM-* findings appear. Confirm they are absent
      today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Extend capability mapping; populate (or delete) `native_links`.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; the new audit fixtures pass; existing audit
      goldens updated intentionally for the new disclosures.

## Validation Plan

- Regression test(s): the per-capability audit fixtures.
- Runtime proof: `mfb audit` on the reproduction lists network/process/env/clock/
  randomness/native.
- Doc sync: if any AUDIT-PERM-* code changes reachability status, update the
  diagnostics spec.
- Full suite: `scripts/test-accept.sh`.

## Summary

The audit's capability disclosure is half-wired: three of nine capabilities and
the native-links surface never populate, so their finding codes/renderers are
dead. The fix completes the capability mapping (and native-links population or
removal); risk is in the `os` process-vs-environment split.
