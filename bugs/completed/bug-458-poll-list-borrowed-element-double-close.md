# bug-458: `poll(List OF RES Socket)` double-closes the borrowed element it returns

Last updated: 2026-08-30
Effort: small (< 1h)
Severity: HIGH
Class: Resource lifetime (double close / double free)

Status: Fixed (plan-110-E Phase 3)
Regression Test: `codegen::engine::value::builder_values::borrowed_resource_tests::poll_list_forms_alias_a_live_resource`

## Symptom

The list form of `poll` returns a **borrowed** pointer to the first ready element
— the list keeps ownership and closes it exactly once at its own scope exit
(§15.6, plan-76-A). Binding that return therefore registers no close obligation:

```basic
MUT socks AS List OF RES tcp::Socket = []
socks = collections::append(socks, connA)
RES ready AS tcp::Socket = tcp::poll(socks)   ' borrowed, NOT owned
```

In fact the bind was classified as an **owner**, so codegen emitted a
`tcp.close(ready)` cleanup at the bind's scope exit *and* the list drain closed
the same handle again. `tls::poll` closes a handle whose record owns arena-held
TLS context memory, so there the second close is a double *free*, not just a
stale `close(2)`.

## Root cause

`CodeBuilder::value_aliases_live_resource` (bug-375's classifier) matched only

```rust
NirValue::Call { target, .. } | NirValue::CallResult { target, .. } => {
    matches!(native_bare_target(target), Some("get" | "getOr")) || target == "net.poll"
        || target == "tls.poll"
}
```

A built-in package member lowers to `NirValue::RuntimeCall`, not `Call`, so the
`net.poll` / `tls.poll` names in that arm were **never reached** — dead conditions
that read as a fix. `collections::get`/`getOr` lower as a plain `Call`, which is
why the collection-element half of bug-375 kept working and hid the gap.

The adjacent, structurally identical `value_is_runtime_managed` in the same file
does list `NirValue::RuntimeCall`; this one was simply missed.

Measured, macOS aarch64, `tests/rt-behavior/tcp/tcp-poll-list-rt` copied to
`/tmp/pollprobe`:

```
$ mfb build -ncode /tmp/pollprobe     # before: 292358 lines of .ncode
$ mfb build -ncode /tmp/pollprobe     # after:  292043 lines  (-315)
```

The 315 lines are the four `poll(List)` binds' close cleanups, one per fixture
function. Adding `"tcp.poll"` to the *name* list alone changed nothing (`diff` =
0 lines) — only adding the `RuntimeCall` variant did, which is what proves the
name arm was dead rather than merely incomplete.

## Why the acceptance fixture did not catch it

`tcp-poll-list-rt` runs a 1200-iteration connect/poll/close loop precisely to
catch a double close, and it passed. The two closes are adjacent — the alias
cleanup runs immediately before the list drain — so no descriptor is opened
between them and the second `close(2)` just returns `EBADF`, which drop-path
cleanup does not surface. It is only observable once something else can claim the
descriptor in between, or (for `tls`) once the record's arena memory is freed
twice. This is the "register/slot/lifetime bugs rarely red a black-box fixture"
case: the guard is a codegen-inspection unit test, not another runtime fixture.

## Fix

Match `NirValue::RuntimeCall` alongside `Call`/`CallResult`, and list the three
live list-poll targets:

```rust
NirValue::Call { target, .. }
| NirValue::CallResult { target, .. }
| NirValue::RuntimeCall { target, .. } => {
    matches!(native_bare_target(target), Some("get" | "getOr"))
        || matches!(target.as_str(), "tcp.poll" | "udp.poll" | "tls.poll")
}
```

`net.poll` is not listed: plan-110-E removed net's transport surface, and
`udp.poll` — which never had a name arm at all — is added because its list form
has the identical borrowed-element contract.
