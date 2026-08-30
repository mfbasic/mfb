# plan-110-B: TCP package extraction

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-110-A

Create the `tcp` package and move the plaintext stream/listener contract out of `net` while
temporarily retaining legacy `net` entry points as migration shims until plan 110-E. The outcome is
that every requested `tcp::*` overload runs with `tcp::Socket`/`tcp::Listener` ownership and the
same proven OS behavior as today's net implementation.

References: plan-110-A; `.ai/compiler.md`; `.ai/resources-packages.md`; `.ai/net-tls.md`;
`src/codegen/builtins/net/{mod.rs,gen_shared.rs,gen_io.rs,gen_poll.rs}`;
`src/syntaxcheck/builtins.rs:BUILTIN_ARG_MODES`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-110-A archived/completed | `ls planning/plan-110-A-* 2>/dev/null` returns no matches | MET — measured 2026-08-29: no matches; the letter is at `planning/completed/plan-110-A-network-contract-and-ping.md` (commit efed7d071). |
| Full Rust suite green | `rustup run 1.96.0 cargo test` | MET — measured 2026-08-29 at plan-110-A's tip with `--no-fail-fast`: 65 binaries, all `test result: ok`, exit 0. Acceptance (1291 tests) and `artifact-gate all` (1750 goldens, 0 diffs) were green in the same state. |

## 1. Goal

- Add the exact `tcp` signatures from the request, including scalar/list poll and String/List Byte
  write overloads, backed by real native execution on every supported target.

### Non-goals

- Do not change timeout conventions, short-read behavior, full-write behavior, backlog semantics,
  or record layout merely because the package/type names change.
- Do not delete legacy `net` members in this letter; plan 110-E owns the atomic consumer cutover.
- Do not keep `readText`/`writeText`: String is an overload of `write`; `read` remains bytes only.

## 2. Current State

The current surface is `net::{connectTcp,listenTcp,accept,read,readText,write,writeText,close,
poll,localAddress,remoteAddress,setReadTimeout,setWriteTimeout}` with resources `net.Socket` and
`net.Listener` (`src/codegen/builtins/net/mod.rs:register`). Native code is split across 18
consumer files outside net/tls that mention these APIs (`rg -l 'net::|tls::' src/codegen/builtins/http
--glob '*.rs' | wc -l` gives 18 for the HTTP package as a whole); plan 110-E migrates them.

~~Verified by reading `src/syntaxcheck/builtins.rs`: `net.close` is special-cased as consuming, so
`tcp.close` must be registered there.~~ — **false, corrected §C1**: that file does not exist, and
consumption is registry-driven off `close_function`. Verified by reading `net/gen_poll.rs`: list
poll returns a borrowed resource identity and therefore must be retyped, not just renamed at
documentation level — done, and exercised by `func_tcp_stream_valid`'s `RES ready AS tcp::Socket`.

## 3. Design Overview

Add `src/codegen/builtins/tcp/` with descriptors and native emitters moved from the TCP portions of
net. Give resources qualified identities `tcp.Socket` and `tcp.Listener`, close functions
`tcp.close` and an internal listener-shaped alias where required. Retain backend record layout and
resource tags unless the registry proves a tag must change. Legacy net descriptors may delegate to
the same lowerers during the transition, but must return legacy resource identities; no unsafe
cross-identity substitution.

This is behavior-changing at the public API and expected to change TCP/package fixtures on all
targets. Core syscall instruction sequences should remain equivalent; any unexpected semantic
golden diff is localized with one objdump before proceeding.

## Phases

### Phase 1 — Package/resource seam

- [x] Register `tcp` in `src/codegen/builtins/mod.rs` and `src/codegen/registry/mod.rs`; add package
      argument inference and consuming `tcp.close` handling in ~~`src/syntaxcheck/builtins.rs`~~
      — that file does not exist (§C1). Registered in both listed files plus
      `is_builtin_import` (so `IMPORT tcp` resolves), a new `RuntimeHelper::Tcp` family, and the
      three per-target `SUPPORTED_RUNTIME_CALLS`/import-planning arms. "Package argument
      inference" turned out to be a real seam after all, just a different one:
      `ARGUMENT_CHECKED_PACKAGES` (§C6).
- [x] Define `tcp.Socket` and `tcp.Listener` resources, cleanup functions, sendability, runtime tags,
      and verifier/link/binary-representation recognition without changing net identities.
      Close routing, verifier and linker recognition all follow from `close_function: "tcp.close"`;
      `Socket` is sendable and `Listener` is not, as in `net`. The runtime tags are deliberately
      REUSED (`RESOURCE_TAG_SOCKET`/`LISTENER`): the tag is a self-describing marker copied
      verbatim on thread transfer, not a dispatch key, and a tcp socket IS the same runtime record.
      No binary-representation row is needed — those map `net.Socket`/`net.Listener` for the
      `.mfp` type table, which plan-110-E repoints when the net identities are removed.
- [x] Add registry/unit tests proving qualified type lookup and lexical-drop close routing.
      Three in `tcp/mod.rs`: qualified lookup + close op resolving to *tcp's* op while net's still
      resolves to net's, the sendable/non-sendable split, and that endpoints use net's shared
      `Address` record rather than a tcp copy.

Acceptance: a minimal package can declare/drop each tcp resource, and move-after-close is rejected.
**MET** — `func_tcp_stream_valid` declares and drops both resources (one explicit `tcp::close`,
the rest by lexical drop) and runs clean. Cross-identity substitution is rejected, which is the
sharper form of the same guarantee: `func_tcp_invalid` pins
`Call to \`tcp.read\` has argument type(s) (net.Socket, Integer), expected Socket, Integer.`
Commit: 008d745c2

### Phase 2 — Constructors, endpoints, close, timeouts

- [x] ~~Move~~/**share** native lowerers for `listen`, `accept`, both `connect` overload families,
      `localAddress`, `remoteAddress`, `close`, and timeout setters into tcp-owned files.
      Shared, not moved — see §C2 for why the physical move belongs in plan-110-E, where it
      happens once instead of twice. Eleven tcp-owned `func_*.rs` descriptors call them.
- [x] Ensure Address parameters/returns use `net.Address`; `connect(address, timeoutMs)` uses the
      shared value without introducing a tcp Address duplicate. Pinned by the
      `tcp_endpoints_use_the_shared_net_address_record` unit test, which also asserts `tcp` declares
      no records of its own. The ergonomic consequence is §C3.
- [x] Tests: valid/invalid fixtures for every overload and real loopback timeout/address behavior.

Acceptance: a tcp loopback server connects, reports both endpoints, times out according to the
language convention, and closes exactly once under explicit close and lexical drop.
**MET** on macOS and Linux — `func_tcp_stream_valid` binds port 0 and reads back the assigned
port, connects by host/port and by resolved `net::Address`, reports both endpoints, and fires a
read timeout. Windows cannot be certified because its TCP loopback is broken *on main* (§C5);
carried to plan-110-F Phase 2 with the repro.
Commit: 008d745c2

### Phase 3 — I/O and readiness

- [x] Implement byte `read`, byte/String `write`, scalar listener/socket poll, list socket poll, and
      full-write/EOF/error semantics using tcp-owned descriptor names and OS aliases. Three code
      forms carry the overload splits — `tcp.connectAddr`, `tcp.writeText`, `tcp.pollList` —
      each selected in `builder_values` and each force-emitted off its base symbol.
      `write`'s split is new: `net` had separate `write`/`writeText` members, so collapsing them
      into one overloaded member moved the choice from the member name to the payload's type.
- [x] Retarget `scripts/check-net-connect-timeout.sh` to tcp and rename it consistently; preserve its
      real blackhole deadline proof and update `scripts/README.md`.
      → `scripts/check-tcp-connect-timeout.sh`; README entry and the citation in
      `net-connect-timeout-convention-rt` updated. Re-run against a live blackhole:
      `PASS: tcp::connect timed out with ErrTimeout in 0s`.
- [x] Tests: cover all overloads, partial writes, maxBytes boundaries, scalar/list readiness,
      timeout 0/positive/negative, closed handles, and empty-list rejection.
      Split across three fixtures because the resolver reports unknown members before type checking
      runs, so the two classes cannot share one: `func_tcp_stream_valid` (behaviour),
      `func_tcp_invalid` (arity/argument types/cross-package identity), and
      `func_tcp_removed_members_invalid` (the deleted `readText`/`writeText`/`connectTcp`/
      `listenTcp` spellings).
- [x] Added task: fix the pre-existing bounded-`accept` bug that made every socket from
      `accept(listener, timeoutMs)` unreadable (§C4), with a RED-checked regression fixture.

Acceptance: loopback transfers binary and UTF-8 String payloads losslessly, poll returns the exact
ready borrowed socket, and the blackhole script proves the requested deadline.
**MET** — `text=hello` and `bytes=4 last=255` (255 proves no sign extension), `list poll=pick`
returns the borrowed socket whose data is then read through it, and the blackhole script passes.
Commit: 008d745c2

## Validation Plan

Run full `cargo test`, tcp runtime fixtures, acceptance, artifact gates and native target proofs.
Regenerate expected codegen drift only. Update descriptor man content and the embedded stdlib spec.
Run both required rustfmt commands after Rust edits.

## Open Decisions

- Migration shims — recommend internal/shared lowerers plus public legacy net descriptors until
  plan 110-E, because an intermediate commit must keep HTTP and all fixtures buildable.

## Corrections

### C1 — Dangling seam citations (inherited from plan-110-A §C4)

This letter's Phase 1 says to "add package argument inference and consuming `tcp.close` handling in
`src/syntaxcheck/builtins.rs`". **That file does not exist.** Close-consumption is entirely
registry-driven now: `RegistryResource::close_function` → `builtin_resource_close_function` →
`close_op_for` → `consumed_resource` (`src/ir/verify/link.rs:929`). Declaring `close_function:
"tcp.close"` on both resources is the whole of the work, and the
`tcp_resources_are_qualified_and_distinct_from_net` unit test pins that it resolves to *tcp's* op
rather than net's.

### C2 — The emitters are shared now and move in plan-110-E, not here

Phase 2 says to "move/share native lowerers … into tcp-owned files". They are **shared**, not
moved: `tcp`'s members call the `lower_net_*_helper` emitters in
`net::{gen_shared, gen_io, gen_poll}`, which were made `pub(crate)` for the purpose.

The emitters name nothing about `net` — they marshal file descriptors and `sockaddr`s, and the
resource identity is decided entirely by the *descriptor* that calls them — so `tcp` gets
identical syscall sequences under `tcp`-owned symbols and `tcp`-owned resource types, which is
exactly the "no unsafe cross-identity substitution" the letter requires.

Moving the files in this letter would mean editing the same ~2,700 lines twice, because plan-110-E
deletes `net`'s transport descriptors anyway and `gen_io.rs` also holds the UDP emitters that
plan-110-C needs and the address/resolver emitters that `net::lookup` keeps. The physical split
therefore happens once, in plan-110-E, when the callers that would have to be repointed are being
deleted regardless. Added as an explicit task to plan-110-E Phase 3.

### C3 — `tcp` users must also `IMPORT net`, and that is language-correct

Splitting the packages introduced an ergonomic cost the letter did not anticipate: because
endpoints stay `net::Address` (as the letter requires — "without introducing a tcp Address
duplicate"), a file that *uses* an address from `tcp::localAddress` / `tcp::remoteAddress` must
`IMPORT net` as well as `tcp`.

This is not a wiring defect and there is no fix short of duplicating the type. `mfb spec language
modules-and-packages` is explicit: **"Imports are not transitive. A package cannot export an
imported package or create re-export chains."** So `Address` is nameable only in a file that
imports the package declaring it.

Measured symptom, and why it is worth documenting rather than leaving to be discovered: the
diagnostic points at the *consumer*, not the missing import —

```
RES client = tcp::connect("127.0.0.1", bound.port, 2000)
     ^ error[2-203-0043 TYPE_UNKNOWN_VALUE]: Initializer for binding `client`
       does not have a known type.
```

`tcp::connect`, `listen`, `read`, and `write` need nothing but `IMPORT tcp`; only the
address-valued members do. Documented on the package and on both address members.

Also corrected: `mfb spec language modules-and-packages` listed 20 built-in packages and was
missing `app`, `astrings`, `audio`, `money`, `process` **and** `tcp`. Now matches
`is_builtin_import` exactly.

### C4 — A pre-existing bug this letter had to fix: bounded `accept` returned an unusable socket

Found while first exercising `tcp::accept`, reproduced identically through `net`, and **confirmed
at main tip** (`f79f6212a`) with a compiler built from a detached worktree — so it predates
plan-110 entirely.

`accept(listener, timeoutMs)` returned a socket that could not be read: every read raised
`ErrTimeout`. The blocking `accept(listener)` was fine.

Cause: the bounded path puts the **listener** into non-blocking mode so a connection lost between
the readiness poll and the accept becomes EAGAIN and re-enters the poll (bug-314 H2). It restores
the listener's flags afterwards — but on macOS/BSD `accept` gives the new socket the listener's
file-status flags, **including `O_NONBLOCK`**, and nothing cleared it. Reads then returned EAGAIN,
which the read helper reports as `ErrTimeout`.

It survived because the failure is **data-dependent**. With nothing between the accept and the
read, the loopback bytes had usually already landed in the socket buffer, so the non-blocking read
found them and succeeded. Insert any work at all — an allocation, a `localAddress` call — and the
read loses that race:

```
connected
allocated 64          ' strings::repeat("x", 64) -- no net call at all
wrote
Error: 7-705-0008     ' Operation did not complete before its deadline
```

Fixed in `lower_net_accept_helper` by clearing `O_NONBLOCK` on the **accepted** fd on the bounded
path, using the listener's saved original flags (`emit_set_nonblocking` reads that slot without
writing it back, so it still holds the pre-modification value). Guarded on the same
`RESTORE_FLAGS_OFFSET` flag the listener restore uses, so the block-forever overload emits
byte-identically.

Regression fixture: `tests/rt-behavior/net/net-bounded-accept-blocking-rt`, RED-checked before the
fix (`bounded = RAISED` / `blocking = hello64`) and green after (both `hello64`). It also asserts
that an explicitly set read timeout still fires, so the fix cannot be "restore blocking" done by
disabling timeouts.

### C5 — Windows TCP is broken on main; certification of `tcp` there belongs to plan-110-F

`tcp` cannot be execution-certified on Windows because the transport it inherits does not work
there today. Proven pre-existing: the same `net` program built by a **main-tip** compiler
(`f79f6212a`) behaves identically to one built here.

Measured on box 2230 (Windows 11, 10.0.26100.9168):

| Program | Result |
|---|---|
| `net::listenTcp("127.0.0.1", 0)` then `localAddress` | binds, but reports **`0.0.0.0`**, not `127.0.0.1` |
| then `connectTcp("127.0.0.1", boundPort, 2000)` | raises `ErrNetworkFailed` (7-707-0003) |
| `net::listenTcp(hostVariable, 0)` where `hostVariable = "127.0.0.1"` | **listen itself raises** |

The two symptoms point one way: the host string is not reaching `getaddrinfo` intact on Windows.
A literal host behaves as though the node were empty — an empty node plus `AI_PASSIVE` is exactly
what binds `0.0.0.0` — and a non-literal host fails outright. macOS and Linux report `127.0.0.1`
and connect successfully for all three shapes.

This is a real defect and is **not** being left to a future plan: plan-110-F Phase 2 owns "fix
every defect found … never leave a target-specific bug for another plan" for exactly this matrix,
and an explicit task carrying this repro has been added there. It is out of scope for *this*
letter, whose acceptance is the tcp contract; recording it here so the trail from discovery to fix
is unbroken.

### C6 — A new package is silently unvalidated until it is added to `ARGUMENT_CHECKED_PACKAGES`

The letter's "add package argument inference" turned out to name a real seam, just not the one it
cited. `src/codegen/builtins/mod.rs:ARGUMENT_CHECKED_PACKAGES` lists the packages whose calls the
shared table checker validates. **Omission is not a compile error and produces no warning** — the
package simply gets no arity or argument-type checking, and every mistake degrades into a bare
`TYPE_UNKNOWN_VALUE` on the binding.

Measured with `tcp` absent from the list, against the identical call shapes `net` already had a
fixture for:

| Call | `net` spelling | `tcp`, before the fix |
|---|---|---|
| `connect(1, 80)` | `TYPE_CALL_ARGUMENT_MISMATCH` | **no diagnostic at all** |
| `connect()` | `TYPE_CALL_ARITY_MISMATCH` | `TYPE_UNKNOWN_VALUE` |
| `connect(h := …, deadline := 1)` | `TYPE_UNKNOWN_ARGUMENT_NAME` | `TYPE_UNKNOWN_VALUE` |

After adding the row, `func_tcp_invalid` reports 9 `TYPE_CALL_ARGUMENT_MISMATCH`, 3
`TYPE_CALL_ARITY_MISMATCH` and 1 `TYPE_UNKNOWN_ARGUMENT_NAME` — including the cross-package
identity rejection that this whole letter turns on:
`Call to `tcp.read` has argument type(s) (net.Socket, Integer), expected Socket, Integer.`

A comment now warns the next person adding a package. `udp` (plan-110-C) needs the same row.

### C7 — The catalog family count is a deliberate drift sentinel

`catalog_is_consistent` asserts the exact number of catalogued runtime-helper families and went red
at "left: 14, right: 13" when `RuntimeHelper::Tcp` landed. That is the test doing its job: a new
family must be acknowledged rather than appearing silently. Updated to 14 with `Tcp` added to the
expected list — not a re-baseline, since the assertion's whole purpose is to force this
acknowledgement. `udp` will move it to 15.

## Summary

The risk is qualified resource identity and cleanup, not the already-proven socket syscalls. This
letter lands tcp without prematurely breaking current HTTP/net consumers.
