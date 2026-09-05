# bug-522: `thread::transfer`'s transferable-type list contradicts the `thread` package intro, and neither says which types *cannot* cross

Last updated: 2026-09-04
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `spikes/api-review/bug-522-transfer-list-stale/` promoted to a `tests/` fixture, plus a registry-vs-prose pin

Two pages state the transferable set, and they disagree.

`mfb man thread transfer`:

> Only some resource types may cross at all: `fs::File`, `tcp::Socket` and
> `udp::Socket` may; **listeners and `tls::Socket` may not**.

`mfb man thread` (the package intro):

> **Every** built-in socket and listener may cross — `fs::File`, `tcp::Socket`,
> `udp::Socket`, `tcp::Listener`, `tls::Socket` and `tls::Listener` — so a
> server may accept on one thread and hand each connection to a worker.

The registry settles it. All six carry `sendable: true`
(`tcp/mod.rs:180,199`, `tls/mod.rs:177,239`, `udp/mod.rs:171`,
`fs/mod.rs:165`), and `src/codegen/resource/mod.rs`'s own test asserts it,
with the comment: *"bug-464: every transport and file handle moves across
threads. The Listener asserted `!sendable` here until then."* The intro is
current; the `transfer` page was not updated when bug-464 landed.

The contradiction is not cosmetic. The intro's list is what motivates the
package's headline pattern — "accept on one thread and hand each connection to
a worker" — which is exactly the listener/TLS case the `transfer` page forbids.
A reader who trusts `transfer` will architect around a restriction that does
not exist. The `transfer` page also links `tls::Socket` in its **See also**
immediately after telling you it may not cross.

Separately, neither `transfer` nor `accept` lists the types that genuinely
**cannot** cross, so the one thing a reader needs — "is *my* resource
transferable?" — is answerable only by trial.

The single correct behavior a fix produces: one statement of the transferable
set, correct against the registry, present on `transfer` and `accept`, listing
both what may cross and what may not, with the reason for each exclusion.

References:

- `src/codegen/builtins/thread/func_transfer.rs` — the stale list and the
  contradictory See-also
- `src/codegen/builtins/thread/mod.rs:91` — the intro's correct list
- `src/codegen/resource/mod.rs:107` `is_builtin_sendable_resource_type`, and
  the `builtins_carry_close_op_and_sendability` test below it
- `bugs/completed/` — bug-464, which opted the listeners in
- Spike: `spikes/api-review/bug-522-transfer-list-stale/`

## Failing Reproduction

```
./target/release/mfb man thread transfer   # "listeners and tls::Socket may not"
./target/release/mfb man thread            # "Every built-in socket and listener may cross"

./target/release/mfb build spikes/api-review/bug-522-transfer-list-stale
./spikes/api-review/bug-522-transfer-list-stale/build/mfb_project.out
```

- Observed:

```
a tcp::Listener crossed a thread boundary; the worker returned 1
=> `mfb man thread transfer`'s transferable-type list is stale.
```

  The spike declares `Thread OF RES tcp::Listener TO Integer`, transfers a real
  listener from `tcp::listen`, and the worker `thread::accept`s it. If the
  `transfer` page were right this would not compile.

  `tls::Socket` and `tls::Listener` type-check on a resource channel too,
  verified by compiling a program declaring both
  (`Thread OF RES tls::Socket TO Integer` and
  `Thread OF RES tls::Listener TO Integer`).

- Expected: the `transfer` page names all six, and the two pages agree.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ (prose defect; the `sendable` bit is target-independent) |

## Root Cause

The transferable set has exactly one source of truth: the `sendable` field on
each `RegistryResource`, read by
`src/codegen/resource/mod.rs:is_builtin_sendable_resource_type` and enforced by
the IR verifier (`src/ir/verify/resources.rs:require_thread_sendable`).

bug-464 flipped `tcp::Listener` (and the `tls` pair) to `sendable: true` and
updated `thread/mod.rs`'s intro. `func_transfer.rs`'s `DESC` restates the same
set in prose and was missed. Registry prose is `&'static str` that the compiler
never reads, so nothing connected the flipped bit to the sentence describing it
— exactly the drift `.ai/man-content.md` warns `mfb man` output is the only
check for.

The **See also** list is generated from the descriptor's cross-references, so
`tls::Socket` appearing there is a second, independent trace of the same stale
edit.

## Goal

- `mfb man thread transfer` and `mfb man thread accept` state the same
  transferable set as `mfb man thread`, and that set matches the registry.
- Both pages list the built-in resources that may **not** cross, each with its
  reason.
- A test fails if a `sendable` bit is flipped without the prose following.

### Non-goals (must NOT change)

- Any `sendable` bit. The registry is correct; six types cross and five do not.
  This bug changes prose and adds a pin, nothing else.
- The `THREAD_SENDABLE` opt-in for user-declared resources, which the intro
  documents correctly.
- The `See also` mechanism.
- **Tempting wrong fix, forbidden:** "resolving" the contradiction by flipping
  `tls::Socket`/`tls::Listener`/`tcp::Listener` back to `sendable: false` so the
  `transfer` page becomes true. bug-464 opted them in deliberately, with a
  record-tail audit behind it, and the spike proves the capability works today.

## Blast Radius

The complete built-in resource census — `grep -rn "RegistryResource {"
src/codegen/builtins/` returns 11 rows, and `grep -rn "sendable:"` gives each
verdict:

**May cross (`sendable: true`) — 6:**

| type | declared at |
| --- | --- |
| `fs::File` | `fs/mod.rs:165` |
| `tcp::Socket` | `tcp/mod.rs:180` |
| `tcp::Listener` | `tcp/mod.rs:199` |
| `udp::Socket` | `udp/mod.rs:171` |
| `tls::Socket` | `tls/mod.rs:177` |
| `tls::Listener` | `tls/mod.rs:239` |

**May not cross (`sendable: false`) — 5, with the reason each row records:**

| type | declared at | reason |
| --- | --- | --- |
| `process::Process` | `process/mod.rs:228` | owns the child's pipe fds and drives `waitpid` from its owning thread; `waitpid` semantics are per-thread on some platforms |
| `audio::AudioInput` | `audio/mod.rs:408` | a capture stream is driven from its owning thread (blocking read / OS callback ring) |
| `audio::AudioOutput` | `audio/mod.rs:427` | a playback stream blocks on write from its owning thread |
| `canvas::Image` | `canvas/mod.rs:965` | belongs to the drawing surface's thread |
| `canvas::Font` | `canvas/mod.rs:986` | belongs to the drawing surface's thread |

Sites:

- `src/codegen/builtins/thread/func_transfer.rs` — fixed by this bug.
- `src/codegen/builtins/thread/func_accept.rs` — **must be checked**: if it
  restates the set, it carries the same staleness and is fixed here too.
- `src/codegen/builtins/thread/mod.rs:91` — already correct; gains the
  may-not-cross list.
- `src/codegen/builtins/tcp/mod.rs:276` — already asserts sendability in a
  unit test; the model for the new pin.
- `.ai/canvas-threading.md`, `.ai/resources-packages.md` — check whether either
  restates the transferable set; both are version-controlled docs that drift
  the same way.

## Fix Design

Three parts.

1. Replace the stale sentence in `func_transfer.rs` with the correct six, and
   add the may-not-cross table with reasons. Mirror it onto `func_accept.rs`.
2. Add the may-not-cross list to `thread/mod.rs`'s intro, which currently says
   only what *may* cross — the reader's question is usually the negative one.
3. Add the pin that would have caught this: a test that walks every
   `RegistryResource` and asserts its `sendable` bit against an explicit
   expected list, so flipping a bit without updating this bug's table fails.
   That is the mechanical half; the prose half cannot be gated, so the test's
   failure message should name the man pages to update.

Rejected: writing the transferable list into the prose *generated* from the
registry, so it cannot drift. Attractive, but the registry's prose fields are
`&'static str` literals with no interpolation, so generating this sentence
means a renderer change well beyond the scope of a stale list.

Rejected: deleting the list from `transfer` and pointing at the package intro.
The list is exactly what a reader of `transfer` is looking for; a redirect
makes the common case worse to save a duplication that a test can guard.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land `spikes/api-review/bug-522-transfer-list-stale/` (done).
- [ ] Add the registry pin: every built-in resource's `sendable` bit asserted
      against the two tables above. It passes today — it is the guard, not the
      red test.
- [ ] Read `func_accept.rs` for a restatement of the set; record the verdict.
- [ ] `grep -rn "may cross\|thread-sendable" .ai/` — record whether the
      version-controlled topic docs restate it.

Acceptance: the pin passes and names both tables; `accept` and the `.ai/` docs
have verdicts.
Commit: —

### Phase 2 — the fix

- [ ] Correct `func_transfer.rs`'s list; add the may-not-cross table with reasons.
- [ ] Mirror onto `func_accept.rs` and `thread/mod.rs`.
- [ ] Fix any `.ai/` doc found in Phase 1.

Acceptance: `mfb man thread`, `mfb man thread transfer` and
`mfb man thread accept` state the same set, matching the registry; the See-also
on `transfer` no longer contradicts its own body.
Commit: —

### Phase 3 — validation

- [ ] `scripts/man-run-examples.sh thread --run`.
- [ ] `scripts/man-census.sh --memory-scope`.
- [ ] `cargo test --no-fail-fast -- thread resource` — scoped; this is prose
      plus one pin.

Acceptance: examples run; census clean; the pin is green.
Commit: —

## Validation Plan

- Regression test: the registry-vs-expected-list pin, which fails if a
  `sendable` bit moves.
- Runtime proof: `spikes/api-review/bug-522-transfer-list-stale/` — a
  `tcp::Listener` crossing a real thread boundary.
- Doc sync: `func_transfer.rs`, `func_accept.rs`, `thread/mod.rs`, and any
  `.ai/` topic doc restating the set.
- Full suite: scoped to `thread`/`resource` plus the man harness.

## Open Decisions

- Whether the may-not-cross reasons should be sourced from a new
  `unsendable_reason` field on `RegistryResource` rather than duplicated as
  prose. **Recommend the field** if a second page ever needs the list; for now
  the duplication is two pages and the pin catches drift.

## Summary

The fix is prose plus a test, with no behavior change: the registry has been
right since bug-464 and the spike proves the capability works. The value is in
the pin — this exact drift has already happened once silently, and the census
tables above give the pin something concrete to assert against.
