# bug-173 — builtins/syntaxcheck type-checking nits (unsound function-param variance, unvalidated named args, loose arity, overloaded-signature leaks, dead branches)

Last updated: 2026-07-12
Severity: LOW (batch); item A is a real (low-severity) type-soundness hole.
Class: Correctness / Footgun / Dead-code.
Status: FIXED (2026-07-13; goal: resolve bugs 170-180; full acceptance suite green)

## Findings

**A. Function-value parameter compatibility is covariant (should be
contravariant) — admits an unsound callable assignment.**
`src/syntaxcheck/types.rs:140`. The `Function`/`Function` arm compares parameters
with `self.compatible(expected_param, actual_param)`. Passing `FUNC(A)` where
`FUNC(AB)` (union) is expected checks `compatible(AB, A)`, which the User/User
union arm (`:158-166`) accepts because `A` is a variant of `AB`. So a function
accepting only `A` is accepted into a slot promised to be callable with any `AB`,
and may then be invoked with a `B`. Return type is checked in the sound
direction; the hole is limited to union-typed function parameters. (Unclear
whether `ir::verify` re-checks function-value compatibility.) Fix: compare
parameters contravariantly — `self.compatible(actual_param, expected_param)`.

**B. Named argument silently accepted for builtins lacking param-name metadata.**
`src/syntaxcheck/builtins.rs:1777-1782`. When a call has named args but
`builtins::call_param_names(callee)` is `None` and there are no overloads, args
bind by source order and no `TYPE_UNKNOWN_ARGUMENT_NAME` is emitted (contrast the
`Some(param_names)` path at :1805). A typo'd/reordered name is silently mis-bound.
Fix: reject `CallArg::Named` for builtins with no param-name metadata.

**C. `math::resolve_call` accepts wrong arity for ABS/MIN/MAX.**
`src/builtins/math.rs:144`. The arm `ABS | MIN | MAX if all_same_numeric(.., 1, 2)`
allows 1..=2 for all three, so `min`(1 arg)/`abs`(2 args) resolve a return type
instead of `None`. Harmless only because a separate `arity()` gate runs first.
Fix: split into `ABS if ..1,1` and `MIN|MAX if ..2,2`.

**D. `net::argument_types` returns a concrete signature for the overloaded
timeout setters.** `src/builtins/net.rs:285`. `setReadTimeout`/`setWriteTimeout`
are overloaded on `Socket|UdpSocket` but return a fixed `"Socket, Integer"`
(doc at :272-275 says overloaded calls must return `None`). Harmless today (first
operand is always a resource var, never a literal). Fix: return `None` for the
two timeout setters.

**E. Internal lowered-only call names are user-reachable via `is_*_call`.**
`src/builtins/thread.rs:49` and `src/builtins/tls.rs:37`.
`thread.emitResource`/`transferResource`/`acceptResource`/`readResource` and
`tls.closeListener` (synthesized only during IR lowering) are reported by
`is_thread_call`/`is_tls_call` (→ `is_builtin_call`) as real builtins, so a
user-typed `thread.emitResource(x)` gets a "builtin with bad arguments"
diagnostic instead of "unknown function" (net/regex exclude their internal names).
Fix: recognize these only in the post-lowering classifier.

**F. Resource-union thread-sendability is vacuously true.**
`src/syntaxcheck/resources.rs:281`. In `is_thread_sendable_type_with_seen`, a
`Type::User` that is a resource union falls to the `Union` arm, which does
`variants.iter().all(|v| v.fields.iter().all(...))`; resource-union variants carry
empty `fields`, so `.all()` over empty is vacuously true and the underlying
resources' `is_sendable` bits are never consulted — a non-sendable resource
variant can ride the thread plane without `TYPE_THREAD_NOT_SENDABLE`. Fix: when a
variant name is itself a registered resource, gate on
`resource_registry.is_sendable(&variant.name)`.

**G. Dead/no-op branches (batched).** `src/syntaxcheck/builtins.rs:1064-1068`
(term checker indexes `arg_types[index]` relying on `min==max==param_types.len()`
from term.rs — would OOB if a term builtin ever had optional args; use `zip`);
`:926-930` (io arity `if min==0 {"0"} else {min.to_string()}` — inner branch
produces identical output); `src/syntaxcheck/checking.rs:169-174, 223-227,
276-278, 283-285, 292-294, 571-573` (empty `if <pure-condition> {}` residue from
plan-20-Z relocations — pure conditions, no observable effect).
