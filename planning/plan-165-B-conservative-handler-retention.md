# plan-165-B: Prove infallibility before removing handlers

Last updated: 2026-10-03
Effort: medium (estimate: 1h–2h)
Depends on: plan-165-A
Status: Not started; implementation awaits the owner's go-ahead

## Goal

Missing error declarations or unknown argument types must never establish that a
handler is dead. Retain the handler and actual error capture whenever proof is absent.

## Prerequisites

Plan-165-A must be complete before implementation (`rg -n '^Commit:|^- \[ \]'
planning/plan-165-A-handler-contracts.md`, or its archived path), and an explicit
owner go-ahead to implement this plan (for example `/follow-plan plan-165-B`).
Reviewing or discussing the plan is not one.

## Current state and evidence

`src/codegen/builtins/mod.rs:inline_builtin_is_infallible` grants infallibility to
an inline-native member whose declared errors are empty, unless raw-supported.
`inline_builtin_raw_supported` consults those declarations too
(`sed -n '370,495p' src/codegen/builtins/mod.rs`).
`src/codegen/registry/mod.rs:native_member_declares_error` reads registry lists,
not emitted failure paths (`sed -n '3334,3370p' src/codegen/registry/mod.rs`).
The toString argument rule recognizes List OF Byte as fallible but currently gives
unknown argument types the infallible name-based answer
(`sed -n '285,320p' src/codegen/builtins/mod.rs`).

Bug-797 (commit a5324104b) added a third consumer of the same proof:
`src/ir/verify/trap_init.rs` calls `inline_builtin_is_infallible` (with
`NoTypeKinds`) to decide whether a call is a failure edge into a function-level
handler, and therefore whether that handler may read a later-declared local
(`TYPE_TRAP_LOCAL_UNINITIALIZED`); `trap_init::total_functions` builds project-function
totality on top of it (`rg -n 'inline_builtin_is_infallible' src/ir/verify/trap_init.rs`).
An unsound grant there accepts a handler read of a local that a failure skipped.

These are unsafe proof mechanisms. This document does not claim a specific current
empty-metadata builtin has an undeclared runtime failure; establish such a repro
before filing that as an observed miscompilation.

## Design

Separate capability to capture a failure from proof that no failure can occur.
An AbiInline body already has a capture wrapper through
`src/codegen/engine/value/builder_values.rs:lower_inline_builtin_raw`; permit that
path for unclassified inline-native members even if their error lists are empty.

Use an explicit implementation-grounded proof set for total primitive operations.
Audit each retained proof against its lowering, including all overloads. Missing
proof defaults to fallible. Metadata may invalidate a proof by declaring an error;
absence of metadata cannot grant a proof. Unknown arguments cannot justify an
overload-specific proof, especially toString's byte-decoding distinction.

Test safety at the oracle, IR and runtime levels. Also audit
`src/ir/fallible.rs:analyze` and `src/ir/verify/trap_init.rs`: function-body
inference and the bug-797 failure-edge census must both use conservative answers,
including locals referenced from function-level handlers. Audit reporting may be
less precise, but it must not be mistaken for the optimizer's proof source.

Correctness risk: merely retaining a warning/handler in IR is insufficient if
native lowering still unconditionally wraps the operation as successful.

## Non-goals and compatibility

Do not remove useful proven optimizations wholesale, change declared error sets to
hide missing proof, or add a runtime fallback. ABI and package format stay unchanged.
Unproved handlers may make binaries larger, as explicitly preferred by the owner.
Source compatibility also changes on purpose: a builtin that loses its proof
becomes a failure edge in the bug-797 census, so a function handler that reads a
local declared after such a call becomes `TYPE_TRAP_LOCAL_UNINITIALIZED`. That is
the correct verdict (the read was unsound), not a regression; fix affected
fixtures by moving the declaration before the call, as bug-797 did for bug-465's
tcp/tls EOF fixtures, and list every such fixture in the commit.
Byte identity is not the correctness criterion for intentionally retained checks.

## Phases

### Phase 1 — Audit and RED proof tests

- [ ] Enumerate `inline_builtin_is_infallible` and raw-capture consumers with
      `rg -n 'inline_builtin_is_infallible|inline_builtin_raw_supported' src`
      (expect `src/ir/verify/trap_init.rs` among them).
- [ ] Record an implementation proof for each retained total primitive/overload.
- [ ] Add failing tests that empty metadata alone and Unknown toString arguments
      cannot grant infallibility; verify unproved members remain raw-capable.
- [ ] Add runtime byte-decoding failure cases through handler locals and wrappers;
      capture any observed defect in its own bug document.
- [ ] Add a verifier test (alongside bug-797's in `src/ir/verify/tests.rs`) that an
      unproved builtin call is a failure edge for handler-local readability.

Acceptance: new oracle regressions fail for the documented unsafe proofs before
implementation; audit lists every granting path (estimate: minutes plus source audit).
Commit: —

### Phase 2 — Conservative decisions and specification

- [ ] Replace the blanket metadata grant with audited positive proofs.
- [ ] Route all unproved inline-native members through real raw capture.
- [ ] Verify unknown overload types remain fallible through function inference.
- [ ] Update error-model dead-handler prose to describe a correctness invariant,
      not an accepted loss of catchability; document proof ownership in code.
- [ ] Add valid/invalid fixtures and run builtin oracle plus runtime regressions.

Acceptance: `cargo test --bin mfb inline_builtin` and proposed runtime recovery
cases pass, with live failure capture verified (estimate: minutes after build).
Commit: —

## Open decisions

None about conservatism: absent proof retains the handler. Exact primitive proof
membership must come from the audit rather than assuming every empty list is total.

## Validation discipline

New runtime tests execute compiled programs and assert stdout, error codes and
side-effect order. Add matching valid/invalid fixtures under the repository's
`tests/rt-behavior/trap` and `tests/syntax/trap` trees. Do not weaken existing
assertions: record history, protected behavior, consumers and an independent
contract/reproduction proving each changed expectation wrong.

Run formatting and scoped tests during implementation. At the end of this
sub-plan, run `cargo test --no-fail-fast` and
`scripts/test-accept.sh target/debug/mfb target/accept-error-model` before any
existing golden correction; full cargo testing includes the artifact gate.
These broad checks are required by the repository and may take hours (estimate).
After justified corrections, rerun affected checks and required final gates.
Verify specs with `cargo build`, `cargo test --bin mfb spec`, and rendered topics.

## Corrections

Planning only. An initial implementation attempt was removed at the owner's
instruction; no compiler or test changes are retained. Any experimental success
is not acceptance evidence for a future implementation.

2026-10-03 review: bug-797 landed after this plan was written and made
`src/ir/verify/trap_init.rs` a consumer of `inline_builtin_is_infallible`. Added it
to the audit, the design and a verifier test, and recorded the intended
`TYPE_TRAP_LOCAL_UNINITIALIZED` source-compatibility change. Replaced the
session-specific authorization text with a reusable go-ahead condition.
