# bug-525: `tcp` and `tls` disagree on double-close and on the `listen` backlog default, and the docs ratify the split

Last updated: 2026-09-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: **FIXED** (`9053e5eb0`) — every built-in `close` refuses an already-closed
handle with `ErrResourceClosed`, and both `listen` members default `backlog` to
`128`. The divergence was not a decision waiting to be made: `mfb spec language
resource-management` §15 had already made it, and `tls`/`audio` were the
non-conforming side.
Regression Test: `tests/rt_double_close_is_refused.rs` (runtime, cross-transport)
+ `codegen::resource::tests::every_builtin_close_refuses_an_already_closed_handle`
(lowering, all six backends) +
`codegen::resource::tests::both_listen_members_default_the_same_backlog`

`tcp` and `tls` are documented as drop-in mirrors of each other — same member
names, same shapes, same argument order — and they diverge in two places.

**1. Double-close.** `tcp::close` on an already-closed handle raises;
`tls::close` succeeds.

`src/codegen/builtins/tcp/func_close.rs:34`:

> **An already-closed handle is an error rather than a no-op, and `tls::close`
> deliberately differs** — there, closing twice succeeds. The two are otherwise
> drop-in mirrors, so the split is worth knowing: code moved between the
> transports must not assume either answer. **Neither package will change under
> the other without a decision, because each has callers relying on what it
> does today.**

**2. `listen` backlog default.** `tcp::listen` defaults `backlog` to `128`;
`tls::listen` defaults it to `0`, meaning "use the host default"
(`tcp/func_listen.rs:120`, `tls/func_listen.rs:130`). Both pages name the other
and say the two transports do not queue the same depth unless the argument is
given explicitly.

Both divergences are known, deliberate, and documented — which is genuinely
better than the usual outcome, where one side gets "fixed" and the other
silently breaks. But "documented and deferred" is not "resolved". The sentence
*"neither package will change under the other without a decision"* names a
decision that has never been made, and until it is, every author porting code
between the two transports carries two special cases.

The single correct behavior a fix produces: the decision is made and written
down. Either the two converge (with the losing side's callers migrated), or the
divergence becomes a stated, principled rule rather than an accident of
history.

References:

- `src/codegen/builtins/tcp/func_close.rs:34-46` — the double-close paragraph
- `src/codegen/builtins/tls/func_close.rs:27-38` — the idempotent side
- `src/codegen/builtins/tcp/func_listen.rs:23-30`, `:120` — backlog `128`
- `src/codegen/builtins/tls/func_listen.rs:22-27`, `:130` — backlog `0`
- `.ai/net-tls.md`

## Failing Reproduction

The divergences are documented, so the reproduction is reading the two pages
side by side:

```
./target/release/mfb man tcp close   | grep -A4 "already-closed"
./target/release/mfb man tls close   | grep -A2 "idempotent"
./target/release/mfb man tcp listen  | grep -A3 "backlog"
./target/release/mfb man tls listen  | grep -A3 "backlog"
```

- Observed: `tcp::close` "An already-closed handle is an error rather than a
  no-op"; `tls::close` "The call is idempotent with respect to a socket that is
  already closed". `tcp::listen` backlog "defaults to `128`"; `tls::listen`
  "Defaults to `0`, which uses the host default."
- Expected: one answer per question across the two transports, or a written
  rule explaining why the answers differ.

A runtime reproduction is deferred to Phase 1 rather than written now: the
`tls` half needs a certificate and key pair, and the per-process TLS port gate
makes a naive fixture flaky. Phase 1 owns building it properly.

Contrast case, correct today: the `tcp::close` page is explicit that the
difference is invisible to the recommended idiom — "close once, or let the end
of the scope do it" — since the automatic close after an explicit one does
nothing on both. So the blast radius is *code that closes twice on purpose*,
which is the population Phase 1 must actually count.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | documented divergence, not yet measured — Phase 1 |
| Linux / Windows | — | the TLS backends differ per platform (`.ai/net-tls.md`); the backlog is ignored on macOS, so measure all three |

## Root Cause

Not a defect in either implementation — two independently-correct choices that
were never reconciled.

`tcp::close` treats the closed flag as a state machine: a second close is a
use-after-close and is refused, with the same reasoning as any other `tcp::`
call on a closed handle. Its page argues this explicitly — "the handle is
marked closed either way, so a second attempt is refused rather than acting on
a socket that may by then belong to something else."

`tls::close` treats close as an assertion of a post-condition, so repeating it
is harmless.

The backlog split has a similar shape: `tcp` picked a concrete portable default
(`128`); `tls` deferred to the host, which on macOS is moot because
Network.framework manages its own accept queue (`tls/func_listen.rs:25`).

## Goal

- A single documented rule for double-close across `tcp`, `tls`, `udp`, `fs`
  and `audio` — not just the two that are contrasted today.
- A single documented rule for `listen`'s backlog default, or an explicit
  statement of why the transports differ.
- Whichever way each is decided, a cross-transport test pins it so the two
  cannot drift apart again.

### Non-goals (must NOT change)

- Silently changing either behavior. The `tcp` page states that each side has
  callers relying on today's answer; a change is a migration, not an edit.
- The recommended idiom (close once, or let scope do it), which is correct and
  unaffected either way.
- `tcp::close`'s `ErrResourceMoved` for a transferred handle, and its
  `ErrCloseFailed`-exactly-once rule. Both are good and orthogonal.
- The macOS `tls::listen` backlog behavior (accepted for signature parity,
  ignored by Network.framework) — that is a platform fact, not a divergence.
- **Tempting wrong fix, forbidden:** converging by editing one package's docs
  to match the other's behavior. The divergence is in the code; a doc-only
  "fix" would make one page a lie.

## Blast Radius

The question is broader than the two packages the docs contrast. Every builtin
`close` needs a verdict — from `grep -rn 'name: "close"' src/codegen/builtins/`:

| member | double-close today | to determine in Phase 1 |
| --- | --- | --- |
| `tcp::close` | raises | — (documented) |
| `tls::close` | succeeds | — (documented) |
| `udp::close` | **unknown** | read `udp/func_close.rs` |
| `fs::close` | **unknown** | read `fs/func_close.rs` |
| `audio::close` | **unknown** | read `audio/func_close.rs` |
| `process::close` | idempotent on the pipe | different operation entirely — see bug-524 |

Backlog: only `tcp::listen` and `tls::listen` take one.

Other sites:

- `.ai/net-tls.md` — records the transport contracts; follows whatever is decided.
- `examples/`, `benchmark/` — count call sites that close twice deliberately.
- `repository/` — uses the network stack; check whether it relies on either answer.
- `src/docs/spec/**` — gated by nothing; grep for a statement of close semantics.

## Fix Design

This is a decision document as much as a fix. The engineering is small; the
work is establishing what the rule should be and migrating whichever side
loses.

**Double-close.** The two defensible rules:

- *Idempotent everywhere* (the `tls` answer). Close is an assertion; a second
  one is a no-op. Simplest for callers, and it composes with scope-exit drop
  without a special case. Cost: `tcp` loses a genuine use-after-close signal.
- *Raising everywhere* (the `tcp` answer). Close is a state transition; using a
  closed handle is a bug regardless of which call you use. More consistent with
  every *other* `tcp::`/`tls::` member, all of which raise on a closed handle —
  which is the stronger argument, because it makes `close` stop being special.

**Recommend raising everywhere**, on the consistency-with-siblings argument,
*if* Phase 1 finds few deliberate double-closers. If it finds many, idempotent
is the cheaper migration and is also defensible. The count decides.

**Backlog.** `tcp`'s explicit `128` is the better default: it is portable,
predictable, and does not vary by host. Aligning `tls::listen` to `128` changes
behavior only where the host default was smaller, and on macOS it changes
nothing (the argument is already ignored). This one is cheap and low-risk.

Rejected: leaving both and improving the cross-references. That is the current
state, and the current state is what this bug records.

Rejected: adding a project-level configuration switch. Two behaviors selected
at build time is worse than two behaviors selected by package name.

## Phases

### Phase 1 — measure, then decide (no behavior change)

- [ ] Read `udp/func_close.rs`, `fs/func_close.rs`, `audio/func_close.rs` and
      fill in the double-close column above with the *code's* answer, not the
      page's.
- [ ] Write a runtime fixture per package that closes twice and records the
      result, so the table is measured rather than read. The `tls` case needs a
      cert/key pair and must respect the per-process TLS port gate.
- [ ] `grep -rn "close" examples/ benchmark/ repository/ tests/` for
      deliberate double-closes; count them.
- [ ] Record the two decisions in this file with their rationale.

Acceptance: the table has a measured verdict per member; the double-close call
sites are counted; both decisions are written down.
Commit: —

### Phase 2 — converge

- [ ] Apply the double-close decision to whichever members diverge from it.
- [ ] Align `tls::listen`'s backlog default with `tcp::listen`'s.
- [ ] Migrate the call sites Phase 1 found.
- [ ] Rewrite the paragraphs in `tcp/func_close.rs`, `tls/func_close.rs`,
      `tcp/func_listen.rs` and `tls/func_listen.rs` that exist only to explain
      the divergence.

Acceptance: every builtin `close` gives the same answer to a double-close;
both `listen` members default the same; the explanatory paragraphs are gone.
Commit: —

### Phase 3 — pin it + validation

- [ ] Add a cross-transport test that asserts the *same* double-close answer
      across every builtin `close`, so a future divergence fails a test rather
      than earning a paragraph.
- [ ] Regenerate goldens; `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] Run the network fixtures on Linux and Windows — per the project rule, a
      per-backend change is not proven by lowering.
- [ ] Update `.ai/net-tls.md`.

Acceptance: full suite green on all three platforms; the cross-transport pin
is the thing that would have caught this.
Commit: —

## Validation Plan

- Regression test: the Phase 3 cross-transport double-close pin, plus a backlog
  test asserting the same default from both `listen` members.
- Runtime proof: the per-package fixtures from Phase 1, re-run after Phase 2.
- Doc sync: four `func_*.rs` pages, `.ai/net-tls.md`, `src/docs/spec/**`.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`, on macOS,
  Linux and Windows.

## Open Decisions

- Idempotent vs. raising, resolved by the Phase 1 call-site count.
  **Recommend raising** on the consistency argument, with the count as the veto.
- Whether `fs::close` is in scope. It drains a buffer before closing, so a
  second close has a different meaning there. **Recommend including it** —
  a rule with an exception nobody can remember is not a rule — but expect
  `fs` to need its own sentence about the drain.

## Summary

The engineering is small and the decision is the deliverable. The risk is that
Phase 1 finds a meaningful population of deliberate double-closers, in which
case the recommended direction flips — which is exactly why the count comes
before the change. Nothing here is urgent; what makes it worth doing is that
the deferral is currently permanent by default.

## Resolution (2026-09-05, `9053e5eb0`)

**Which side was wrong: the code, on the `tls` and `audio` side.** The report
framed this as a decision nobody had made. It had been made — by the language
spec, before either package diverged from it.

`mfb spec language resource-management` §15 (`15_resource-management.md:22`):

> A resource is still closed **exactly once**. What changed is *who* may close
> it, never *how many times*: an already-closed record is flagged, and a second
> close is a defined no-op reported as `ErrResourceClosed` rather than an
> operation on a dead handle.

That is the `fs`/`tcp`/`udp` behaviour verbatim. `tls::close` and `audio::close`
set the closed flag once and then returned OK forever after, so they were the
non-conforming side rather than a second defensible policy — and
`src/docs/spec/stdlib/17_transports.md:54` ratifying the split ("`tls::close`
differs deliberately") was the **stdlib** spec contradicting the **language**
spec. That sentence is corrected here.

### Phase 1 — the measured table

Taken from a program, not from the pages: open a handle, close it through a
`RES` parameter, close it again the same way, print the second call's code.

| member | double-close before | after |
| --- | --- | --- |
| `fs::close` | `ErrResourceClosed` (77030004) | unchanged |
| `tcp::close` | `ErrResourceClosed` | unchanged |
| `udp::close` | `ErrResourceClosed` | unchanged |
| `tls::close` (Socket **and** Listener) | **success** | `ErrResourceClosed` |
| `audio::close` (Input **and** Output) | **success** (lowering; no device on any host) | `ErrResourceClosed` |
| `process::close` | a different operation; renamed `process::closeInput` by bug-524 | — |

### Phase 1 — the count that was the veto

**Zero.** A literal double close does not compile:

```
tcp::close(l)
tcp::close(l)
   ^ error[2-203-0055 TYPE_USE_AFTER_MOVE]: binding is used after move
```

so the only reachable shape is a close through a `RES` parameter or another
alias — §15's "any holder may close it". None of the 219 `*::close` call sites
under `examples/`, `benchmark/`, `repository/`, `tests/` and `packages/` does
that, and the three `tls::close` sites in `examples/` already wrap the call in
`TRAP … RECOVER`. So the recommended direction (raise everywhere) stands
un-vetoed, and it is also the direction the spec already required.

### The decisions, written down

1. **A second close raises `ErrResourceClosed`, in every built-in package.** Not
   a per-package choice: `close` is not an exception to "using a closed handle
   is a mistake", which is what every *other* member of these packages already
   says. The recommended idiom is unaffected — close once, or let the scope do
   it — because a drop-close discards its failure (§15).
2. **`listen`'s `backlog` defaults to `128` on both transports.** `tcp`'s
   explicit, portable default wins over `tls`'s "host default"; there is now one
   constant, `builtins::net::DEFAULT_LISTEN_BACKLOG`, read by the code-layer
   padding and by the `tls::listen` descriptor. Raising a backlog can only make
   the kernel queue *more* pending connections, so no program that worked stops
   working, and macOS ignores the argument entirely.

### The trap in the emitters

Ten close emitters. **Three had the SUCCESS path falling through into the
`already` label** and sharing its OK tag (`gen_schannel_read_close`, both audio
macOS closes, ALSA, WASAPI). Turning that label into a refusal without first
inserting `abi::branch(&done)` makes the *first* close fail. That is what the
positive pin below exists to catch, and it is recorded in `.ai/net-tls.md`.

### Tests

- **RED, runtime, cross-transport** — `tests/rt_double_close_is_refused.rs`.
  `tls::Listener` printed `second=0` before the fix; `fs`, `tcp` and `udp`
  printed `77030004` before *and* after, so they are positive pins rather than
  three more copies of the same assertion.
- **RED, lowering, cross-backend** —
  `codegen::resource::tests::every_builtin_close_refuses_an_already_closed_handle`
  lowers all ten emitters and asserts each relocates
  `_mfb_str_error_resource_closed`. It is the only instrument that reaches
  Schannel and WASAPI from a Mac, and the only one that reaches `audio` at all.
- **POSITIVE pin** — `closing_once_and_letting_the_scope_end_is_unchanged`:
  open a `tcp::Listener` and a `tls::Listener`, use each, close each **once**,
  then rebind the released port. The rebind is what proves the handle is still
  closed exactly once; `main` returning 0 with both still in scope is what
  proves a drop-close still discards its failure.
- **Backlog pin** — `both_listen_members_default_the_same_backlog`.

### Per-backend runtime proof

Matched before/after runs of the same program, built by the pre-fix and post-fix
compilers. Lowering is not runtime proof for a per-backend change:

| backend | `tls::Listener` | `tls::Socket` |
| --- | --- | --- |
| macOS / Network.framework | `0` → `77030004` | `0` → `77030004` |
| Linux / OpenSSL (box 2228) | `0` → `77030004` | `0` → `77030004` |
| Windows / Schannel (box 2230) | `0` → `77030004` | `0` → `77030004` |

`fs`, `tcp` and `udp` printed `77030004` on every box both before and after. The
`tls::Socket` runs complete a real handshake — against `openssl s_server` on
macOS and Linux, and against the program's own listener on a worker thread on
Windows, where there is no `openssl` CLI. **`audio` has no runtime proof on any
host** (no device on the Mac or on the boxes); its instrument is the lowering
pin, and that gap is stated rather than implied.

### Golden delta

The padded backlog constant `0` → `128` in four `.ir` goldens
(`byte-identity/tls`, `syntax/tls/{accept,close,listen}_valid`), and the
`.ncodesum` of exactly the four fixtures that embed a changed emitter:
`byte-identity/{tls,audio,http,resource-xfer-slots}`. **Zero `.run` goldens
moved and no `build.log` moved** — the behaviour change is only reachable
through a shape no fixture had.

### Also fixed, in scope

Every built-in `close` now declares its raisable errors, so the derived Errors
table states the rule the pages describe: `fs::close` (`ErrResourceClosed`,
`ErrResourceMoved`, `ErrCloseFailed`, `ErrWriteFailed`), `tcp`/`udp::close`
(the first three), `tls::close` (`ErrResourceClosed`, `ErrTlsFailed`),
`audio::close` (`ErrResourceClosed`, `ErrAudioUnavailable`). All five declared
`errors: vec![]` before, so none rendered an Errors table at all. The sets are
taken from each emitter's `emit_fail` calls, not from its prose.

### Open Decisions — resolved

- **Idempotent vs. raising** → raising, as recommended. The call-site count that
  was the veto is zero, and §15 had already settled the direction.
- **Whether `fs::close` is in scope** → yes; it already conformed, and it is the
  precedent that a raising close is safe against the drop path. Its pre-close
  drain needed no extra sentence: the drain runs before the closed-flag guard is
  ever reached a second time.

### Non-goals honoured

`tcp::close`'s `ErrResourceMoved` for a transferred handle and its
`ErrCloseFailed`-exactly-once rule are untouched — a moved `tls` handle still
reports `ErrResourceClosed` rather than `ErrResourceMoved`, which the report
listed as orthogonal and out of scope. The recommended close-once idiom is
unchanged. macOS's ignored `tls::listen` backlog is still ignored. No page was
edited to match the other package's behaviour: the divergence was in the code,
and the code is what moved.
