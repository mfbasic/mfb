# plan-116-I: `setGroup` takes ownership of the resources in its list

Last updated: 2026-08-31
Effort: medium (1h–2h)
Depends on: plan-116-H, **and plan-114 (A–E) complete**

The feature request specifies that `canvas::setGroup` *"takes ownership of any
resources in the list (post plan-114, a `Picture` holds a `RES Image`); the group owns
them until it is dropped."*

That behaviour is **not implementable today and this letter does not attempt it**.
`TYPE_RESOURCE_FIELD_FORBIDDEN` (`2-203-0084`) is live — `src/rules/table.rs:993`, with
no `RETIRED` marker, unlike the rule at `:1001` — so a record field cannot hold a
resource, and every `DrawItem` variant names its image and font through the plain value
handles `ImageRef` and `FontRef` (`mod.rs:385-423`). `mod.rs:385` states the
consequence: *"This is what keeps the scene from retaining anything."*

So in plan-116-G, `setGroup` takes ownership of nothing **because nothing ownable can
be in the list** — a vacuous truth, not a shortcut. This letter is what makes it a real
one, once plan-114 has made `Picture` able to hold a `RES Image`.

Behavioral outcome: a program opens an image, puts it in a `Picture` inside a
`setGroup` list, and lets its own binding go out of scope — and the image stays usable
for as long as the group is installed, closing exactly once when the group is replaced
or removed and no frame still draws it. Doing that 200 times in a loop does not exhaust
file descriptors or leak the backing texture.

References:

- `planning/plan-114-D-lift-the-ban.md` — the letter that retires `2-203-0084`.
- `planning/plan-114-B-record-res-slot-codegen.md`,
  `planning/plan-114-C-escape-record-edges.md` — the layout and ownership routing this
  letter's group buffer must participate in.
- `.ai/canvas-threading.md` §7 — the closed flag and the deferred texture free, which
  this letter must compose with rather than duplicate.
- `.ai/resources-packages.md` — the RES resource system.
- plan-116-G §4.2–4.3 — the group table, the deep copy, and the refcount + drain gate.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates.

| Must be true | Command | Status |
|---|---|---|
| plan-116-H complete and archived | `ls planning/completed/plan-116-H-*` → one match | NOT MET |
| **plan-114 A–E complete and archived** | `ls planning/plan-114-*` → **no matches** | NOT MET (5 matches) |
| The ban on resource record fields is retired | `grep -n 'TYPE_RESOURCE_FIELD_FORBIDDEN' src/rules/table.rs` shows a `RETIRED` comment | NOT MET |
| `Picture` holds a `RES Image` | `grep -n 'RES Image\|ImageRef' src/codegen/builtins/canvas/mod.rs` | NOT MET — **and plan-114 will not change this**, see below |

**Row 4 will still be NOT MET the day plan-114 lands.** plan-114 does not touch
`canvas` anywhere: `grep -rln 'canvas' planning/plan-114-*.md` returns **no files** —
zero mentions across all five letters — and letter D's fixtures use `fs::File`.
Confirmed independently by mfb-76, the session executing plan-114, 2026-08-31.

So when plan-114 completes, rows 2 and 3 flip to MET and **row 4 does not**. The
`Picture`-from-`ImageRef`-to-`RES Image` migration is entirely ahead of this letter and
belongs to nobody yet. Phase 1 and this letter's effort estimate must be written on
that basis — see §Open Decisions, where it is no longer a conditional.

**If plan-114 is not complete, this letter cannot start, full stop.** It is not scope
this letter absorbs, not a soft preference, and there is no dual-mode design in which
`setGroup` owns a handle today and a resource later. plan-116-G ships the group feature
in full without it; this letter adds ownership when the language supports it.

The fourth row is a separate gate from the second on purpose: retiring the *rule* makes
`RES` fields legal, but `Picture` migrating from `ImageRef` to `RES Image` is a
`canvas` change that plan-114 does not make. If plan-114 lands and `Picture` still holds
an `ImageRef`, this letter's first phase is that migration — see §Open Decisions.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report the
> status of **all four** rows if you stop.

## 1. Goal

- `setGroup` takes ownership of every resource reachable from the item list it is
  given, so the caller's bindings may go out of scope without closing them.
- The group closes each owned resource exactly once, when the group's buffer is freed
  — which plan-116-G §4.3 already gates on `refs == 0 AND retiredFrame <
  lastCompletedFrame`.
- A resource owned by a group and still drawn by an in-flight frame is not closed until
  that frame completes.
- 200 install/remove cycles leak neither file descriptors nor backing textures.

### Non-goals (explicit constraints)

- **`present` does not take ownership.** A published scene retaining resources would
  reverse `mod.rs:385`'s design and `func_present.rs`'s documented promise that
  *"an installed scene never keeps an image open"*. Only `setGroup` owns, because only
  a group has a lifetime longer than one `present`.
- **No change to `canvas::destroyImage` / `destroyFont` semantics.** Closing a resource
  a group owns follows the existing closed-flag model (`.ai/canvas-threading.md` §7);
  this letter adds *who closes it*, not *when the backing is freed*.
- **No new `canvas::` surface.** `setGroup`'s signature is unchanged.
- **No change to group storage, lifetime accounting, resolution or GPU rendering** —
  plan-116-G and -H.

## 2. Current State

**This section must be re-measured when the letter starts**, because it describes a
world (post-plan-114) that does not exist yet. What is recorded here is the state at
the time of writing, so a future implementer can see what changed.

### Measured, 2026-08-31

| What | Value | Command |
|---|---|---|
| `TYPE_RESOURCE_FIELD_FORBIDDEN` live | yes | `sed -n 991,996p src/rules/table.rs` — no `RETIRED` comment, unlike `:1001` |
| plan-114 letters open | 5 (A–E) | `ls planning/plan-114-*` |
| plan-114 letters archived | 0 | `ls planning/completed/plan-114-*` → no matches |
| `Picture.image` type | `ImageRef` (a record wrapping an `Integer`) | `mod.rs:647-668`, `:397-410` |
| `Text.font` type | `FontRef` | `mod.rs:609-646` |
| Resources declared by `canvas` | 2 (`Image`, `Font`) | `mod.rs:730-826` |
| `live_slots` on both | `&[]`, `sendable: false` | `mod.rs:748`, `:790` |

### Verified properties

- **The two `canvas` resources are not transfer-audited.** Read `mod.rs:744-748`:
  `sendable: false`, `live_slots: &[]`, with the comment *"Not audited for transfer
  (bug-464 left canvas out of scope). Empty here is only consistent with
  `sendable: false`; opting an image in means auditing its record tail first, not just
  flipping the bit."* A group is worker-owned state that the graphics thread *reads*,
  so this letter must establish whether group ownership constitutes a transfer under
  plan-114's rules. **UNVERIFIED and it is the letter's first task.**

- **A `Picture` holding a `RES Image` cannot cross a thread data plane, and after
  plan-114-A that is a hard compile error rather than a silent acceptance.**
  plan-114-A adds `2-203-0137 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`
  (`planning/plan-114-A-thread-plane-resource-error.md:182`; the rule does **not** yet
  exist on main — `grep -rn '2-203-0137' src/rules/table.rs` returns nothing). Combined
  with `canvas::Image`'s `sendable: false`, a record carrying one is refused at any
  thread boundary.

  **This is a design input, not a discovery to be made during implementation.** If any
  part of this letter — or of a program using it — expects a `Picture`, a group's item
  list, or a `List OF DrawItem` containing one to be sendable, that has to be designed
  in deliberately, which means auditing `Image`'s record tail and setting `live_slots`
  rather than flipping `sendable`. Raised by mfb-76, 2026-08-31; the rule code is
  `-0137`, not the `-0138` first reported.

  Note this does **not** obstruct plan-116-G's design: the group table is process-global
  storage read by the graphics thread, which is not a thread *data plane* transfer in
  the `thread::start` sense the rule governs. Whether the type system agrees is exactly
  what Phase 1's audit must settle.
- **`.ai/canvas-threading.md` §7's gate already defers the OS-side free** past any
  in-flight frame. So "close on group free" composes with it: the group's close sets
  the closed flag, and the existing gate frees the backing. This letter should add
  *no* new deferral mechanism — see §3.

## 3. Design Overview

Three pieces, and the whole letter is deliberately small because plan-116-G already
built the hard part:

1. **Establish whether group ownership is a "transfer"** under plan-114's rules, and
   audit both `canvas` resources' record tails if it is. §2's verified-properties note.
2. **`setGroup`'s deep copy takes the resources with it** — the copy already exists
   (plan-116-G §4.2 reuses `gen_present.rs:emit_publish`); post-plan-114 it must route
   ownership per plan-114-C's escape-record-edges rules rather than copying a handle.
3. **The group's free path closes what it owns**, immediately before releasing the
   buffer, inside the gate plan-116-G §4.3 already implements. **No new deferral**: the
   close sets the closed flag and the existing texture gate
   (`.ai/canvas-threading.md` §7) does the rest.

**Where the correctness risk concentrates:** double-close and use-after-close across
the worker/graphics boundary. plan-59-B's runtime backstop makes a second close a
defined `ErrResourceClosed` rather than corruption, which bounds the damage — but a
group closing an image a *scene* still names would make that scene draw nothing, which
is a silent wrong picture. The rule that prevents it is already written:
`.ai/canvas-threading.md` §7 says a `Picture` carries a value handle, so *"presenting a
stale one draws nothing rather than raising"* — post-plan-114 that sentence changes and
**must be re-derived, not assumed**.

**Byte-identity is NOT this letter's gate.** **Expected NOT to diff:** every canvas
golden — this letter changes ownership, not pixels. **Expected to diff:** `.ncodesum`
on every canvas-emitting target.

### Rejected alternatives

- **Refcount the resources themselves alongside the group's own refcount.** Rejected:
  the RES model deliberately has no refcount (`.ai/canvas-threading.md` §7), and adding
  one for group-owned resources only would give the subsystem two ownership models for
  the same object depending on where it is stored.
- **Have `present` own resources too, for symmetry.** Rejected in §Non-goals: it
  contradicts a documented promise and would make a published scene keep an image open.
- **Copy the resource (dup the fd / clone the texture) into the group.** Rejected: it
  is not what ownership means here, it would double every image's memory, and
  `canvas::Image` has no defined clone.

## 4. Detailed Design

**Deliberately deferred.** The design below the level of §3 depends on decisions
plan-114-B and plan-114-C make about record `RES` slot layout and escape-edge routing,
and writing it now would be writing against a guess. **The first task of Phase 1 is to
read plan-114-B/C/D as landed and fill this section in** — that is a task, not an
omission, and it is listed as one.

What can be fixed now, because it does not depend on plan-114:

- The close happens on the **worker**, in the group free path plan-116-G §4.3 placed at
  the top of `present`. Not on the graphics thread: an arena is per-thread and a
  cross-thread free corrupts the worker's free list (`.ai/canvas-threading.md` §3).
- The close happens **before** the buffer is released, so the resource handles are
  still readable.
- The close is **one per owned resource per group buffer**, so a group replaced by
  `setGroup` closes the old buffer's resources and not the new buffer's.

## Compatibility / Format Impact

- **Behavioural change to `setGroup`:** resources in the list are owned by the group.
  A program that relied on closing them itself after `setGroup` gets
  `ErrResourceClosed` on the second close — a defined, trappable outcome (plan-59-B),
  not corruption.
- **No signature change**; no new type or member.
- **`.ncodesum` churn.**

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — Re-measure, and finish this document's §4

The letter opens against a world that did not exist when it was written.

- [ ] Re-run every row of §Prerequisites and every row of §2's measured table; update
      both in place.
- [ ] Read plan-114-B, -C and -D **as landed** and write §4's detailed design against
      them.
- [ ] Settle the §2 open question: does installing a resource into a process-global,
      graphics-thread-readable group buffer constitute a transfer under plan-114's
      rules? If yes, audit `Image`'s and `Font`'s record tails and set `live_slots`
      accordingly (`mod.rs:748`, `:790`) — *"opting an image in means auditing its
      record tail first, not just flipping the bit."*
- [ ] Re-derive `.ai/canvas-threading.md` §7's last paragraph: post-plan-114, does a
      `Picture` still carry a value handle, or a resource? The sentence *"presenting a
      stale one draws nothing rather than raising"* is either still true or must be
      rewritten. **Do not assume it survived.**

Acceptance: §4 of this document is written against landed code, §2's table is current,
and the transfer question has a recorded answer with the audit behind it.
Commit: —

### Phase 2 — Ownership on the way in

- [ ] `setGroup`'s deep copy routes resource ownership per Phase 1's design instead of
      copying a handle.
- [ ] Tests: a program that opens an image, `setGroup`s a `Picture` naming it, drops its
      own binding, and presents the group — the image still draws.

Acceptance: the drop-the-binding case draws the image; `cargo test --no-fail-fast`
green; every canvas golden byte-identical.
Commit: —

### Phase 3 — Ownership on the way out (largest blast radius)

- [ ] The group free path closes each owned resource once, on the worker, before
      releasing the buffer.
- [ ] `setGroup` replacing a live group closes the **old** buffer's resources only.
- [ ] Tests, extending plan-116-G Phase 5's race matrix — add the rows to
      `.ai/canvas-threading.md` §8 as well:
      - group owning an image → `removeGroup` → graphics mid-frame: the frame completes
        and still samples the texture.
      - the same, then a completed frame: the image closes exactly once.
      - a group owning an image, and a *scene* also drawing that image: assert the
        documented outcome (Phase 1's re-derivation decides what it is) and that it is
        not a crash.
      - `setGroup` replacing a group: the old resources close, the new ones do not.
      - 200 × install/remove of a group owning a `Font` and an `Image`: file descriptors
        and `groupBytes=` return to baseline.

Acceptance: all five rows pass; the 200-cycle loop shows no fd growth (`lsof` on the
process, or the platform equivalent) and no `groupBytes=` growth;
`cargo test --no-fail-fast` green on mac+RELEASE and linux+DEBUG.
Commit: —

### Phase 4 — Docs and gates

- [ ] `mod.rs` — `setGroup`'s description says the group keeps the images and fonts in
      its list usable for as long as it is installed, and that you do not close them
      yourself. **No memory vocabulary** — not "own", "free", "release", "refcount".
      The permitted words are copy, mutate, value, and alias-for-RES
      (`.ai/man-content.md`); `scripts/man-census.sh --memory-scope` → 0 unclassified
      hits.
- [ ] `src/docs/spec/app/06_canvas.md` §"Images are named, not embedded" — the group
      exception to *"a published scene never keeps an image open"*.
- [ ] `.ai/canvas-threading.md` — §7's re-derived paragraph and §8's new rows.
- [ ] `scripts/man-run-examples.sh canvas --run` passes.
- [ ] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.

Acceptance: `cargo test --no-fail-fast` green on both axes, `scripts/test-accept.sh`
green, `scripts/artifact-gate.sh all` 0 diffs, and `mfb man canvas setGroup` describes
the lifetime in observable terms with zero memory vocabulary.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_canvas_graphics_thread.rs` (race matrix ×5),
  `tests/cli_canvas_image_resource.rs` (ownership + double-close),
  `tests/rt_canvas_present_deep_copy.rs`. Negative cases: closing a group-owned image
  yourself (defined `ErrResourceClosed`, per plan-59-B); a group owning an already-
  closed image.
- **Coverage check:** confirm the close path is in the denominator — a group free that
  never runs in the suite would leave this entire letter untested while green. The
  200-cycle loop is what forces it.
- **Runtime proof:** the 200-cycle install/remove loop with fd and `groupBytes=`
  measured before and after.
- **Doc sync:** `src/docs/spec/app/06_canvas.md`, `.ai/canvas-threading.md` §7 and §8,
  `setGroup`'s description.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **`Picture` must migrate from `ImageRef` to `RES Image`, and it is nobody's job
  yet.** No longer conditional: plan-114 provably does not touch `canvas`
  (`grep -rln 'canvas' planning/plan-114-*.md` → no files), so the migration will still
  be outstanding when plan-114 completes. Two options, and this needs deciding **before**
  this letter is scheduled rather than during it:
  - *(recommended)* a **separate lettered plan** between plan-116-H and this one,
    because the migration is a breaking `canvas` API change on its own terms — it
    touches every `Picture[` site, the scene deep copy, `canvas::imageRef`, and
    `.ai/canvas-threading.md` §7's "a `Picture` carries a value handle, so presenting a
    stale one draws nothing rather than raising", which stops being true. Bundling that
    into an ownership letter hides a second breaking change behind the first.
  - as Phase 1.5 of this letter, which keeps the letter count down but makes this
    letter x-large and puts two independent breaking changes behind one acceptance gate.

  Either way **re-estimate before starting**: this letter is written as medium on the
  assumption the migration is not in it.
- **Is a group-owned resource a "transfer"?** Genuinely open; Phase 1 answers it with
  an audit. Recommend assuming **yes** until the audit says otherwise, because the
  buffer is process-global and read by a second thread, which is the shape `sendable`
  exists to govern.
- **What happens when a scene draws an image a group owns and the group is removed?**
  Phase 1 re-derives it. Recommend preserving today's observable outcome — the item
  draws nothing rather than raising — because that is what
  `.ai/canvas-threading.md` §7 documents and what a program can already encounter.

## Corrections

<!-- Filled in during execution. -->

## Summary

This letter is small in code and unusually large in preconditions. The feature request
describes it in terms of a world that does not exist — `Picture` holding a `RES Image`
— and the honest treatment is a hard prerequisite on plan-114 rather than a fallback,
a dual-mode design, or quietly implementing the vacuous version and calling it done.
plan-116-G already ships groups in full, including the lifetime gate this letter hooks
into; what is added here is only *who closes the resources and when*. The parts worth
real care are two: whether a process-global, second-thread-readable group buffer counts
as a transfer under plan-114's rules — which decides whether `Image` and `Font` need the
record-tail audit `mod.rs:748` says they have never had — and whether
`.ai/canvas-threading.md` §7's "presenting a stale handle draws nothing" survives
`Picture` becoming a resource. Both are scheduled as Phase 1 reading tasks, because
both are assumptions that would otherwise be inherited silently.
