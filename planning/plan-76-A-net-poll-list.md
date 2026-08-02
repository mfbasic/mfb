# plan-76-A: net::poll(List OF Socket) AS Socket

Last updated: 2026-08-02
Overall Effort: huge (>3d) — the whole plan-76 feature (non-blocking I/O + async HTTP)
Effort: large (3h–1d)
Depends on: nothing (this is the anchor sub-plan; it holds the feature-wide Prerequisites)

## The plan-76 feature (context for all letters)

plan-76 adds the non-blocking I/O surface that a URL-transparent, cooperatively-driven
HTTP client needs, now that a resource union can carry uniform STATE (plan-74, landed):

- **A (this file)** — `net::poll(List OF Socket) AS Socket`: readiness-multiplex over many
  plaintext sockets. The spec already documents this overload; the code stubbed it out.
- **B** — `tls::poll(sock[, timeoutMs]) AS Boolean`: the first TLS readiness primitive
  (none exists today), backend-specific because a TLS socket buffers decrypted records.
- **C** — `tls::poll(List OF TlsSocket) AS TlsSocket`: the TLS multiplex, building on B.
- **D** — the `http` async client: a `Stream` resource union (`net::Socket | net::TlsSocket`)
  carrying a `PendingState`, driven by `http::startRead / ready / pump / done / finish`, with
  `http::read`/`http::write` rewritten as blocking wrappers over them.

Letter order equals implementation order; each lands before the next. Real dependencies:
C depends on B; D depends on B (its `pump`/`ready` need TLS readiness) and on plan-74. A is
independent of B/C/D and lands first as the lowest-risk primitive.

The single behavioral outcome of **this** sub-plan: a program can build a `List OF RES Socket`
of several connected sockets and call `net::poll(socks)` to block until one is readable (or
`net::poll(socks, timeoutMs)` to bound the wait), receiving back a pointer to the first ready
socket — the list retains ownership and still closes every socket exactly once on scope exit.

References (read first):

- `.ai/compiler.md` — runtime completion gate, validation/function tests, register lifetimes.
- `.ai/specifications.md` — keep the embedded spec current with every compiler change.
- `planning/completed/plan-73-A-timeout-convention-and-thread.md` and `plan-73-C-net.md` —
  the timeout convention this overload obeys, and the net migration that shaped `net::poll`.
- `src/target/shared/code/net/poll.rs:17` (`lower_net_poll_helper`) — the single-socket native
  poll this overload generalizes (the `pollfd`/`poll(2)`/EINTR-retry/sentinel scaffolding to reuse).
- `mfb spec language resource-management` §15.6 (resources in collections) — the ownership-float
  rules that make a returned borrowed-socket pointer sound.

## Prerequisites (feature-wide; B/C/D point back here)

| Must be true | Command | Status |
|---|---|---|
| Working tree builds & tests green at HEAD | `cargo test --bin mfb` (full suite) | MET — 3750 passed; 0 failed (2026-08-02, worktree P-76) |
| Codegen artifact baseline clean | `scripts/artifact-gate.sh target/debug/mfb all` → diffs=0 | IN PROGRESS — full `all` gate running as baseline (see Corrections C1: command needs the `all` selector now) |
| No competing in-flight edits to `src/builtins/{net,tls,http}.rs`, `src/builtins/{net,http}_package.mfb`, `src/target/shared/code/{net,tls}/**` | `git status` on those paths | MET — fresh worktree forked from main; tree clean |
| `List OF RES <resource>` compiles and runs (resources-in-collections landed) | fixtures exist: `ls tests/rt-behavior/resources/resource-collection-floats-runtime` | MET (verified — see Verified properties) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every
> command and update every status before you continue and before you stop. If you stop, report
> the status of *all* prerequisites, not just the one that blocked you.

Everything below is written against the world where these hold.

## 1. Goal

- `net::poll(socks AS List OF RES Socket) AS RES Socket` — blocks until at least one socket in
  `socks` is readable, then returns a pointer to the first ready one (lowest list index).
- `net::poll(socks AS List OF RES Socket, timeoutMs AS Integer) AS RES Socket` — the same, bounded
  by the plan-73 timeout convention: omit = block; `> 0` = wait up to that long; `< 0` =
  `ErrInvalidArgument`; expiry with none ready raises `ErrTimeout` (this is a *producing* call —
  it yields a resource and has no not-ready value to return, per the plan-73-A classification).
  `0` = one immediate scan (raises `ErrTimeout` if none ready).
- The returned pointer aliases a list element; ownership stays with the list's scope. Closing,
  returning, or `thread::transfer`ing through the returned binding is the caller's right (§15.6),
  and the list's later drop finds it already closed (defined no-op). No socket is closed by `poll`.
- An empty list is rejected `ErrInvalidArgument` (nothing to wait on).
- The existing single-socket `net::poll(sock AS Socket[, timeoutMs]) AS Boolean` is unchanged.

### Non-goals (explicit constraints)

- **No writability/poll-mode selection.** Readiness means `POLLIN` (readable / EOF / error),
  matching the single-socket overload. No `POLLOUT` overload is added here.
- **No change to the single-socket overload**, its `Boolean` return, or any other `net::` function.
- **No new resource type or STATE.** `Socket` stays a plain opaque resource.
- **No `List OF Socket` without `RES`.** The argument is `List OF RES Socket`; a bare `List OF
  Socket` is rejected exactly as `LET s AS Socket` is (`TYPE_RESOURCE_REQUIRES_RES`, §15.6).
- **No UDP/`Listener` multiplex.** Only `Socket`.

## 2. Current State

- **Single-socket poll exists**: `net::poll(sock AS Socket[, timeoutMs AS Integer]) AS Boolean`,
  descriptor `src/builtins/net.rs:163` (`nf(POLL, "poll", &[ov(P_POLL, "Boolean")], …)`),
  param array `P_POLL` `src/builtins/net.rs:123`, resolver `src/builtins/net.rs:362-364`, native
  lowering `src/target/shared/code/net/poll.rs:17` (`lower_net_poll_helper`). It builds one
  `pollfd { int fd; short events = POLLIN; short revents; }` on the stack, calls `poll(2)`,
  retries on EINTR (`poll.rs:101-113`), and threads the plan-73 sentinel: omitted timeout is
  padded with `TIMEOUT_UNBOUNDED_SENTINEL` → `poll(-1)` (block); `< 0` → invalid; `> 0` clamped to
  `INT_MAX` (`poll.rs:39-60`).
- **The list overload was stubbed out with a now-stale rationale.** `src/builtins/net.rs:358-361`
  says: *"The `poll(List OF Socket)` overload in the specification is omitted: the ownership model
  forbids resource handles as collection elements, so a `List OF Socket` value cannot be
  constructed and the overload is unreachable."* That statement is **obsolete** — resources in
  collections (`List OF RES Socket`) now compile and run (Verified below). The spec still documents
  the overload; this sub-plan implements it and deletes the comment.
- **The socket fd** lives in the resource record; the single-socket helper reads it at
  `src/target/shared/code/net/poll.rs:61-64` (`FILE_OFFSET_FD` / `FILE_OFFSET_CLOSED`). A
  `List OF RES Socket` element is a pointer to such a record.
- **Resources-in-collections precedent**: `tests/rt-behavior/resources/resource-collection-floats-runtime`,
  `resource-return-collection-order-rt` (a builtin-free `FUNC … AS List OF RES File` returning a
  resource list), `resource-collection-not-owner-valid` (a callee holds pointers it must not close).
- **A builtin returning a resource** is a solved shape (`net::connectTcp AS Socket`,
  `src/builtins/net.rs:151-156`, native `src/target/shared/code/net/mod.rs:1010`) — but every
  existing one *produces* a fresh handle. Returning an *aliased element of a list argument* is the
  novel bit and the design risk (§3).
- **Checker path**: net flows through `check_table_builtin_call` (`src/syntaxcheck/builtins.rs:187`),
  which calls the package `resolve_call` string-matcher — so the new overload is added purely by
  extending `resolve_call` + the descriptor/param tables; no per-function checker edit.

### Measured populations

| What | Count | Command |
|---|---|---|
| `net::poll` overloads today | 1 (Boolean) | `rg -n 'POLL' src/builtins/net.rs` → descriptor :163, resolver :362 |
| Stale "resource handles cannot be list elements" comment | 1 | `rg -n 'poll\(List OF Socket\)' src/builtins/net.rs` → :358 |
| `List OF RES` fixtures proving resource-lists work | ≥3 dirs | `ls tests/rt-behavior/resources \| rg 'collection'` |
| `net::poll` native lowering sites to generalize | 1 | `rg -n 'lower_net_poll_helper' src/target/shared/code/net` → poll.rs:17, mod.rs dispatch |
| net rt-behavior poll tests to mirror | 2 | `ls tests/rt-behavior/net \| rg 'poll'` → `func_net_poll_valid`, `net-poll-timeout-convention-rt` |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `List OF RES Socket` (resource list) compiles & runs | **CONFIRMED** | `tests/rt-behavior/resources/resource-collection-floats-runtime/src/main.mfb:10` binds `MUT handles AS List OF RES File`; the golden `.ir`/`.run` exist and are gated. The mechanism is resource-type-agnostic (`Socket` is a `BUILTIN_RESOURCES` entry like `File`). |
| The net.rs omission comment is stale | **CONFIRMED** | It asserts a `List OF Socket` "cannot be constructed"; the fixture above constructs `List OF RES File`. Same registry path (`resource.rs:151` Socket, `:` File). |
| Single-socket poll's `pollfd`/EINTR/sentinel scaffolding is reusable for N fds | **CONFIRMED (structurally)** | `poll.rs:65-113` builds one `pollfd` and calls `poll(&pfd, 1, timeout)`; generalizing to `poll(&pfds, n, timeout)` is a count + a stride loop over list elements. Read the helper end-to-end before editing. |
| A builtin can return a pointer to an element of a resource-list argument (borrowed-return) | **UNVERIFIED** — Phase 1 falsifies cheaply | No existing builtin returns an aliased argument element. Escape-analysis owner assignment and the `.mfp` resource-region encoding for such a return are unproven. This is the design risk; Phase 1 is the experiment. |
| Empty-list handling | **UNVERIFIED** — Phase 2 | choose reject-`ErrInvalidArgument`; confirm no `poll(&pfds, 0, …)` degenerate call is emitted. |

## 3. Design Overview

Two independent pieces:

1. **Type/resolver surface (Phase 2).** Add a second `net::poll` overload keyed on the argument
   type `List OF RES Socket` → return `RES Socket`, in the descriptor table + `resolve_call`
   (`net.rs`), plus `call_param_names` / `expected_arguments` / `argument_types`. Delete the stale
   comment. The single-socket overload's arms are untouched (different arg type → no ambiguity).

2. **Native lowering (Phase 3).** A new helper (`lower_net_poll_list_helper` in
   `src/target/shared/code/net/poll.rs`) that: reads the list length; rejects length 0
   (`ErrInvalidArgument`); iterates the list elements, loading each element's record ptr and its
   fd (`FILE_OFFSET_FD`) into a stack-allocated `pollfd[n]` with `events = POLLIN`; calls
   `poll(&pfds, n, timeout)` reusing the exact sentinel/clamp/EINTR-retry logic of the scalar
   helper; on return scans `revents` for the first fd with a readiness bit set and returns a
   pointer to that list element's *record* as the `Socket` result value; on expiry (poll==0)
   raises `ErrTimeout`.

**Where design uncertainty concentrates (schedule FIRST):** the **borrowed-resource return** — a
builtin returning a pointer to one of its list argument's elements, without producing or moving a
handle. Phase 1 falsifies this with the smallest possible experiment: a throwaway program that
binds `RES ready AS Socket = net::poll(socks)` (against a temporary hand-wired overload, or against
`collections::get(socks, 0) AS RES Socket` if `get` already returns a borrowed `RES Socket`) and
checks that escape analysis assigns the returned binding to an alias of the list's owner (no double
close, no leak). If the escape/`.mfp` machinery cannot express a borrowed-element return, fall back
to **Open Decision 1** (return an `Integer` index instead of a `Socket`) *before* building Phase 3.

**Where correctness risk concentrates (schedule LAST):** the multi-fd lowering itself
(stack layout of `pollfd[n]`, the list-element stride, the `revents` scan) is the blast-radius
work, guarded by a runtime multiplex fixture and the byte-identity gate. Reuse the scalar helper's
proven poll/EINTR/sentinel block verbatim; only the array build + result selection are new.

**Rejected alternatives:**

- *Return an `Integer` index (first-ready position), caller does `collections::get`.* Safer (no
  borrowed-resource return) but contradicts the spec's documented `poll(List OF Socket) AS Socket`
  and pushes a `get` on every caller. Recorded as the Open-Decision-1 fallback, not the default.
- *Take a bare `List OF Socket`.* Rejected — resources require the `RES` marker in collections
  (§15.6); a bare list is a compile error by design.
- *Re-poll one fd at a time in a loop.* Rejected — defeats the purpose (a single `poll(2)` over N
  fds is the whole point) and changes fairness/latency.

## 4. Detailed Design

### 4.1 Resolver (Phase 2)

- In `net.rs` `resolve_call`, add: `POLL if exact(arg_types, &["List OF RES Socket"]) ||
  exact(arg_types, &["List OF RES Socket", "Integer"]) => Cow::Borrowed("RES Socket")` (confirm the
  exact type-string spelling the checker produces for a resource list — verify against the
  `resolve_call` arg_types seen for `collections::get(List OF RES Socket, …)`; adjust the literal
  to match). Keep the existing scalar arms.
- Add the overload to the descriptor `OV`/`P_*` tables so arity and named-arg metadata resolve
  (`net.rs:123` neighborhood): a `P_POLL_LIST` param array `[req("socks", &[], "List OF RES Socket"),
  opt("timeoutMs", "Integer")]`, and a second `ov(...)` on the `POLL` descriptor with return
  `"RES Socket"`. Update `call_param_names` (`net.rs:296`), `expected_arguments`, `argument_types`.
- Delete the stale comment `net.rs:358-361`.

### 4.2 Native lowering (Phase 3)

- New `lower_net_poll_list_helper` in `net/poll.rs`, dispatched from `net/mod.rs` when the `poll`
  call's first arg is a list (the lowering picks the helper by argument shape, mirroring how the
  other overloaded net calls dispatch).
- Layout: `n = list length`; if `n == 0` → `ErrInvalidArgument`. Allocate `pollfd[n]` on the stack
  (each `pollfd` is 8 bytes: `int fd; short events; short revents`). Loop `i in 0..n`: load element
  `i`'s record ptr from the list payload, load `fd` from `record + FILE_OFFSET_FD`, store into
  `pfds[i].fd`, set `pfds[i].events = POLLIN`, zero `revents`. Reuse `emit_pollfd_events`
  (`poll.rs`) per slot.
- Timeout: identical sentinel/clamp/`< 0`-invalid normalization as the scalar helper (factor the
  shared prologue if clean; otherwise copy the proven block).
- Call `poll(&pfds, n, timeout)`, EINTR-retry (`poll.rs:101-113` pattern). On `ret == 0` (expiry)
  → `ErrTimeout`. On `ret > 0` scan `pfds[0..n]` for the first slot whose `revents & (POLLIN |
  POLLHUP | POLLERR)` is set; return that element's **record pointer** as the `Socket` value.
- Result is a borrowed pointer — emit it as the resource-return value the escape analysis expects
  for a borrowed element (settled in Phase 1). No close, no move.

## Compatibility / Format Impact

- **Changed:** `net::poll` gains a `(List OF RES Socket[, Integer]) → RES Socket` overload; the
  net spec/man page documents it; the stale code comment is removed.
- **Unchanged:** the single-socket overload and every other `net::` function; the `Socket` resource
  type, its close op, and the resource registry; the `.mfp` encoding; the timeout convention.

## Phases

> Tick `- [x]` in the same commit as the work. An unticked box means NOT DONE.

### Phase 1 — falsify the borrowed-resource return (design uncertainty first)

- [x] ~~Write the smallest experiment that a builtin (or `collections::get`) returning `RES Socket`
      aliasing a `List OF RES Socket` element is expressible…~~ — moot: **already proven** by an
      existing precedent, so no throwaway experiment is needed. `RES g AS File =
      collections::get(xs, 0)` on a `List OF RES File` binds a NON-owning alias (bug-375 fix), proven
      at runtime by `tests/rt-behavior/resources/res-rebind-alias-runtime` (peekViaGet / peekViaForEach
      then a post-call use of the still-open handle) and legalised at compile time by
      `tests/syntax/resources/resource-collection-not-owner-valid` (`RES elem AS File =
      collections::get(handles, 0)`, build.log `[exit 0]`). `net::poll(List OF RES Socket) AS RES
      Socket` is structurally identical (returns a pointer to an element the list still owns).
- [x] ~~If it is NOT expressible … STOP and switch to Open Decision 1.~~ — moot: it **is**
      expressible; no fallback needed.

**Decision (YES → `AS RES Socket`):** proceed with the borrowed `RES Socket` return. The single
wiring hook is `value_aliases_live_resource` (`src/target/shared/code/builder_values.rs:192-201`),
which today hardcodes the borrowed-return recognition to the `collections::get`/`getOr` targets. The
list-poll overload is remapped to a distinct NIR target `net.pollList` (mirroring how
`net.connectTcp(Address)` remaps to `net.connectTcpAddr`, `builder_values.rs:1925-1931`); Phase 2/3
teach `value_aliases_live_resource` that `net.pollList` returns a borrowed pointer (generalised via a
`builtins::returns_borrowed_resource(target)` predicate rather than another hardcoded name), so the
`RES ready = net::poll(socks)` bind registers NO close obligation and the list stays the sole owner.
No escape-analysis change and no `.mfp` format change are required (borrow is a pure
lowering-site property; the return serialises as the bare type `Socket`). The ≥1000× leak-loop that
proves no double-close of the borrowed return is folded into the Phase 3 runtime fixture.
Commit: b1fd467c6

### Phase 2 — resolver + descriptor surface

- [x] Added the list overload to `net.rs`: `P_POLL_LIST` (`socks AS List OF RES Socket`,
      `opt timeoutMs`); a second `ov(P_POLL_LIST, SOCKET_TYPE)` on the `POLL` descriptor;
      `resolve_call` arm `exact(["List OF RES Socket"]) || exact(["List OF RES Socket","Integer"])
      => SOCKET_TYPE`; moved `POLL` from `call_param_names` to `call_param_name_overloads`
      (two overloads → `param_names` yields `None`, per the descriptor rule); widened
      `expected_arguments(POLL)`; deleted the stale "overload is unreachable" comment. **Return type
      is `Socket` (bare), not `RES Socket`** — matching `collections::get`, whose resolver strips the
      `RES` axis (`general::list_element` → `"Socket"`); the borrow is a lowering-site property, not a
      return-type spelling (Corrections C2). Updated the two net.rs unit tests
      (`call_param_names_present_and_absent`, `expected_arguments_remaining_arms`).
- [x] Tests: `tests/syntax/net/func_net_poll_list_valid` (accept — both overloads: scalar
      `net::poll(conn)`/`net::poll(conn,100)` → `Boolean`, list `net::poll(socks)`/`net::poll(socks,100)`
      → `Socket`) and `tests/syntax/net/func_net_poll_list_invalid` (reject — bare `List OF Socket`
      → `TYPE_RESOURCE_REQUIRES_RES`; a `String` arg → `TYPE_CALL_ARGUMENT_MISMATCH`; a `String`
      timeout → `TYPE_CALL_ARGUMENT_MISMATCH`). Scalar overload still resolves to `Boolean`.

Acceptance: at `-ast -ir`, the list overload resolves to `Socket`, the scalar to `Boolean`, bad
forms rejected; `cargo test --bin mfb` green (3750 passed, 0 failed). ✅
Commit: 70167a08b

### Phase 3 — native multi-fd lowering (largest blast radius)

- [x] `lower_net_poll_list_helper` in `net/poll.rs` + dispatch in `mod.rs` (`net.pollList`). The
      overload is remapped `net.poll → net.pollList` in `builder_values.rs` by receiver shape
      (`net_poll_is_list_form`), gets a runtime spec (`NET_POLL_LIST_SPEC`, returns `Socket`) in
      `net_specs.rs`, is registered in `catalog.rs` (+ `CODE_LAYER_ONLY_CALLS`), and its helper body
      is force-emitted whenever `net.poll` is present (mirroring `connectTcpAddr`, `mod.rs`). The
      return-type resolution for the overloaded `net.poll` is selected by arg shape in
      `builder_values.rs`. Borrow classification wired via `net::returns_borrowed_resource` →
      `value_aliases_live_resource` (no close obligation on the returned binding). The helper:
      normalizes the timeout (scalar-helper policy verbatim), rejects the empty list
      (`ErrInvalidArgument`), builds a transient `pollfd[n]` in the arena (per-platform stride 8/16
      and events POLLIN/POLLRDNORM), issues one `poll(2)`/`WSAPoll` with EINTR retry, scans `revents`
      for the first ready slot, returns that element's record pointer (borrowed), and `arena_free`s
      the array on every allocated exit path (expiry → `ErrTimeout`).
- [x] Tests: `tests/rt-behavior/net/net-poll-list-rt` — two loopback conns, write to exactly one;
      `net::poll(socks)` returns the readable one (proven both index directions: write-A → "A",
      write-B → "B"); `net::poll(socks, 0)` on two idle → `ErrTimeout`; `net::poll(socks, -1)` →
      `ErrInvalidArgument`; `net::poll([])` → `ErrInvalidArgument`; a **1200× connect/poll/close
      loop** proving no fd leak and no double-close of the borrowed return. Runs clean on
      macos-aarch64 (output `first A / second B / timeout TRUE / negative TRUE / empty TRUE /
      leak-loop survived`, exit 0); passes `test-accept.sh`.
- [x] Goldens: regenerated `byte-identity/net` `.ncodesum` for all five targets. **Proved the delta
      is purely additive** — a detached base (`b1fd467c6`) `.ncode` dump diffed against the current
      one shows ONLY the new `runtime.net.pollList` function inserted (276 lines added, 0 deleted;
      every existing net helper byte-identical). Confirmed debug-gen == release-gen shas for all 5
      targets (`.ncode` is compiler-optimization-independent), so the goldens match the release gate.

Acceptance: the multiplex fixture runs clean and picks the correct ready socket; the timeout cases
match the plan-73 table; 1200× loop leaks no fd; `cargo test --bin mfb` (3750/0) + `net` byte-identity
delta proven additive + release-parity confirmed. (Full `artifact-gate all` deferred to finalization —
a concurrent gate from another session held the global lock; net goldens independently verified.)
Commit: f37003c4c

### Phase 4 — docs

- [x] Man page `src/docs/man/builtins/net/poll.md`: added both list overloads to the Synopsis,
      Overloads, Parameters (`socks`), Return value (borrowed `Socket`), and Errors (`ErrTimeout`
      77050008 for the list expiry; empty-list `ErrInvalidArgument`); replaced the stale "deliberately
      not implemented / unreachable" paragraph with the readiness-multiplex description (borrowed
      pointer, list owns/closes, empty→ErrInvalidArgument, producing-call ErrTimeout); added a
      multiplex example. Fixed a stale `net_connect_is_address_form` citation → `net_poll_is_list_form`.
- [x] Spec `src/docs/spec/language/18_builtin-functions.md`: reconciled the timeout-convention
      classification — the scalar `net::poll` stays a readiness query; the list `net::poll` is now
      listed as a **producing call** (yields the first ready socket, `ErrTimeout` on expiry). The net
      function list already names `net::poll` (one name, two overloads). The plan's claim that the
      spec "already documents the `poll(List OF Socket)` overload" was imprecise — the spec lists the
      function name, not per-overload signatures (those live in the man page); reconciled the
      convention classification instead (Corrections C3).

Acceptance: `mfb man net poll` renders all four overloads; `cargo test --bin mfb` green (3750/0),
man/spec-citation tests pass (5/0). ✅
Commit: 9b6e533e5

## Validation Plan

- Tests: syntax accept/reject (Phase 2); rt-behavior multiplex + timeout + leak-loop (Phase 3).
- Coverage check: the new fixtures exercise the list-arg resolver arm and the multi-fd helper (both
  in the gate denominator via `tests/syntax/net` and `tests/rt-behavior/net`).
- Runtime proof: the two-socket multiplex fixture prints which socket was ready and its bytes.
- Doc sync: `src/docs/man/builtins/net/poll.md`, the net spec section.
- Acceptance: `cargo test --bin mfb`, `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  (net glob), `scripts/artifact-gate.sh target/debug/mfb`.

## Open Decisions

1. **Return `RES Socket` (borrowed element) vs. `Integer` index.** Recommended: `RES Socket`, to
   honor the spec's documented `poll(List OF Socket) AS Socket` overload — *conditional on Phase 1
   proving the borrowed-return is expressible*. Fallback if not: `AS Integer` (first-ready index,
   `-1`/`ErrTimeout` on expiry), which sidesteps the resource-return question entirely.
   Descision: RES Socket
2. **Expiry semantics.** Recommended: treat list-poll as a **producing** call (raise `ErrTimeout`
   on `0`/`> 0` expiry, block on omit) since it yields a resource with no not-ready sentinel — as
   opposed to the scalar `poll`'s readiness-query `Boolean`. (§1)
   Descision: **producing** call

## Corrections

- **C1 (Prerequisites): the artifact-gate command is stale.** The plan (and B/C/D
  which point back here) wrote `scripts/artifact-gate.sh target/debug/mfb`, but the
  gate now *requires* a `<builtin|all>` selector — a bare invocation prints usage and
  runs nothing. Baseline command corrected to `scripts/artifact-gate.sh
  target/debug/mfb all`. Measured via `head -40 scripts/artifact-gate.sh` (the usage
  block). Matches memory note *fast-codegen-gate*.
- **C2 (Current State / Verified properties): the borrowed-resource return is
  EXPRESSIBLE — falsified NO already.** The plan's Phase-1 UNVERIFIED risk ("a builtin
  can return a pointer to a list-argument element as a borrow") is answered YES by an
  existing precedent: `RES g AS File = collections::get(xs, 0)` on a `List OF RES File`
  binds a non-owning ALIAS (bug-375 fix), proven at runtime by
  `tests/rt-behavior/resources/res-rebind-alias-runtime` and legalised by
  `tests/syntax/resources/resource-collection-not-owner-valid`. The plan's cited
  `resource-collection-not-owner-valid` lives under `tests/syntax/resources/`, not
  `tests/rt-behavior/resources/`. Remaining Phase-1 work is narrowed to WIRING
  `net.poll(List)` into the same non-owning-return classification `collections::get`
  uses (so the returned binding registers no close obligation), not proving feasibility.
- **C3 (Phase 4): the spec carries no per-overload `poll` signature to "reconcile".**
  The plan (Phase 4) said `mfb spec` "already documents the `poll(List OF Socket)`
  overload." In fact the net spec (`18_builtin-functions.md`) lists only the function
  *name* `net::poll`; per-overload signatures live in the man page. What needed
  reconciling was the timeout-convention classification: the scalar form is a readiness
  query (`Boolean`/`FALSE`), the new list form is a **producing** call (yields the first
  ready `Socket`, `ErrTimeout` on expiry) — both bullets updated accordingly.

## Summary

The engineering risk is the **borrowed-resource return** (Phase 1 falsifies it before any codegen)
and the **multi-fd lowering** (Phase 3, behind a leak-loop fixture and the byte-identity gate).
Everything else is a thin resolver/descriptor addition plus a doc reconciliation — the scalar
poll's poll/EINTR/sentinel scaffolding is reused verbatim. Untouched: the single-socket overload,
every other `net::` function, the `Socket` type and registry, the `.mfp` encoding, and TLS (B/C).
