# plan-116-J: `setGroup` takes ownership of the resources in its list

Last updated: 2026-09-01
Effort: medium (1h–2h)
Depends on: plan-116-I (which supplies the `RES` fields; plan-114 A–E landed 2026-09-01)

The feature request specifies that `canvas::setGroup` *"takes ownership of any
resources in the list (post plan-114, a `Picture` holds a `RES Image`); the group owns
them until it is dropped."*

That behaviour is not implementable until **plan-116-I** lands: plan-114 retired the
`RES`-record-field ban (2026-09-01; `src/rules/table.rs:1015`, reserved-not-emitted),
but `canvas` itself still names its image and font through the plain value handles
`ImageRef` and `FontRef` until I migrates `Picture`/`Text` to direct
`RES canvas::Image` / `RES canvas::Font` fields and removes those handles.

So in plan-116-G, `setGroup` takes ownership of nothing **because nothing ownable can
be in the list** — a vacuous truth, not a shortcut. This letter is what makes it a real
one, once plan-116-I has put the resources into the items.

Behavioral outcome: a program opens an image, puts it in a `Picture` inside a
`setGroup` list, and lets its own binding go out of scope — and the image stays usable
for as long as the group is installed, closing exactly once when the group is replaced
or removed and no frame still draws it. Doing that 200 times in a loop does not exhaust
file descriptors or leak the backing texture.

References:

- `planning/plan-116-I-canvas-res-handles.md` — the migration this letter is gated on.
- `planning/completed/plan-114-D-lift-the-ban.md` — the letter that retired `2-203-0084`.
- `planning/completed/plan-114-B-record-res-slot-codegen.md`,
  `planning/completed/plan-114-C-escape-record-edges.md` — the layout and ownership
  routing this letter's group buffer must participate in.
- `.ai/canvas-threading.md` §7 — the closed flag and the deferred texture free, which
  this letter must compose with rather than duplicate.
- `.ai/resources-packages.md` — the RES resource system.
- plan-116-G §4.2–4.3 — the group table, the deep copy, and the refcount + drain gate.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates.

| Must be true | Command | Status |
|---|---|---|
| plan-116-H complete and archived | `ls planning/completed/plan-116-H-*` → one match | **MET** (2026-09-04: one match, archived after box 2228 went green) |
| plan-114 A–E complete and archived | `ls planning/completed/plan-114-*` → 5 matches | **MET** (re-verified 2026-09-04: 5 matches, A–E) |
| The ban on resource record fields is retired | `grep -rn TYPE_RESOURCE_FIELD_FORBIDDEN src \| grep -v rules/table.rs` → **no emit site**, only doc comments and the test that pins its absence | **MET** (re-verified 2026-09-04: hits are `ir/verify/tests.rs` ×3, `ir/verify/types.rs` ×2, `ir/verify/resources.rs` ×1 — all doc comments or the pinning tests — plus the spec and the rule-code table; no emit site) |
| **The `canvas::Picture` image sampler exists** — a `Picture` in a scene actually draws its image | `grep -n 'CASE Picture' src/codegen/builtins/canvas/helper_geometry.rs` → the geometry arm is **not** `__canvas_emptyHeader()`; and `grep -rn imageHandle src/ \| grep -v func_handle_bridge.rs` → **at least one renderer call site** | **NOT MET** (2026-09-04, **J11**). The arm is `CASE Picture(pic) RETURN __canvas_emptyHeader()` — the `NONE` kind every renderer skips — and `canvas::imageHandle` has no caller in any renderer, while its twin `fontHandle` has six. Measured: a `Picture` handed straight to `present`, binding alive, renders an all-black frame. Owned by plan-98-E/G, not by any letter of plan-116. |
| **plan-116-I complete and archived** — `Picture` holds a `RES canvas::Image`, `Text` a `RES canvas::Font`, and `ImageRef`/`FontRef` are gone | `ls planning/completed/plan-116-I-*` → one match; `grep -n 'ImageRef\|FontRef' src/codegen/builtins/canvas/mod.rs` → no type declarations | NOT MET |

**The sampler row does NOT block this letter, and saying why matters.** What it blocks is
exactly two *pixel-level* acceptance clauses, which §Phases now states as resource-state
assertions instead — strictly more discriminating for what this letter is about, since a
pixel assertion cannot tell *"owned correctly"* from *"nothing draws"* (**J11**). The
ownership mechanism itself — the transitive move and the free-path close — is fully
implementable and fully testable today against the resource record, the diagnostic and
`groupBytes=`. The row is recorded so that when plan-98-E/G lands, the pixel check is
already written down rather than re-derived, and so nobody reads a green Phase 3 as proof
that a group-owned image survives to the screen.

**If plan-116-I is not complete, this letter cannot start, full stop.** It is not
scope this letter absorbs, not a soft preference, and there is no dual-mode design in
which `setGroup` owns a handle today and a resource later. plan-116-G ships the group
feature in full without it; this letter adds ownership once the items can carry
resources. (The migration was unowned when this letter was first written; the user
directed it into the series as plan-116-I on 2026-09-01.)

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report the
> status of **all five** rows if you stop. *(Five since 2026-09-04: the sampler row is
> **J11**'s.)*

## 1. Goal

- `setGroup` takes ownership of every resource reachable from the item list it is
  given, so the caller's bindings may go out of scope without closing them.
  *(**J9**, 2026-09-04: the second clause is the demanding one and the plan never named
  what it costs. "The caller's binding may go out of scope without closing" means the
  binding must be **moved** — scope-drop closes a `RES` it still owns, and the group
  holding an alias does nothing to stop that. So this bullet requires a transitive move at
  the `setGroup` call site, which nothing implements today. It is not a consequence of
  ownership; it is the mechanism ownership has to be built on. §4.3.1 option 2.)*
- The group closes each owned resource exactly once, when the group's buffer is freed —
  which plan-116-G §4.3 already gates. *(Corrected 2026-09-04, **J4**: the gate has no
  `refs == 0` term and never had one. It is `RETIRED_ITEMS != 0` as the discriminator and
  `frame_now >= stamped` as the frame test, against the frames-**completed** counter.
  `CANVAS_GROUP_REFS` is written and decremented and never read as a predicate anywhere.
  Also **J8**: against a **shared** resource record "exactly once" is satisfied by code
  that is still wrong, because the close is global — see §4.3.1.)*
- A resource owned by a group and still drawn by an in-flight frame is not closed until
  that frame completes.
- 200 install/remove cycles leak neither file descriptors nor backing textures.
  *(**J11**, 2026-09-04: **there are no backing textures yet**, and an image holds no
  descriptor — `canvas::createImage` allocates nothing outside MFB's own resource record,
  and `helper_geometry.rs` gives `Picture` the `NONE` geometry kind that every renderer
  skips. So this bullet is what it will mean once plan-98-E/G lands the sampler. What is
  observable **today** is the arena bytes the group owns, which `groupBytes=` reports, and
  the descriptor a `Font` loaded from a file holds. Both are now in Phase 3; the texture
  half is a Prerequisites row, not a deletion.)*

### Non-goals (explicit constraints)

- **`present` does not take ownership.** Only `setGroup` owns, because only a group has
  a lifetime longer than one `present`. *(Citations corrected 2026-09-04, **J4**: the two
  strings this quoted — `mod.rs`'s *"keeps the scene from retaining anything"* and
  `func_present.rs`'s *"an installed scene never keeps an image open"* — **no longer
  exist in the tree**. The nearest survivor is `src/docs/spec/app/06_canvas.md`'s *"Naming
  a resource in a scene does not keep it alive"*, which plan-116-I wrote. The Non-goal
  itself stands and is now **enforced by a test**:
  `set_group_items_is_the_consuming_parameter` asserts
  `builtin_consuming_parameter_index("canvas.present")` is `None`.)*
- **No change to `canvas::destroyImage` / `destroyFont` semantics.** Closing a resource
  a group owns follows the existing closed-flag model (`.ai/canvas-threading.md` §7);
  this letter adds *who closes it*, not *when the backing is freed*.
- **No new `canvas::` surface** — no new members, and no change to any member's rendered
  signature. *(Corrected 2026-09-04, **J10**. This said *"`setGroup`'s signature is
  unchanged"*, which is true of what `mfb man canvas setGroup` prints and **false of what
  the signature means**: passing an item list now consumes the resources inside it, so a
  second install naming the same image is `2-203-0055` where it used to compile. That is
  a semantic change to a public builtin, it is the change this letter exists to make, and
  a Non-goal phrased to forbid it would have been read as forbidding the letter. What is
  actually ruled out is new surface — and the man-page half of the correction is Phase
  4's.)*
- **No change to group storage, lifetime accounting, resolution or GPU rendering** —
  plan-116-G and -H.

## 2. Current State

**This section must be re-measured when the letter starts**, because it describes a
world (post-plan-114) that does not exist yet. What is recorded here is the state at
the time of writing, so a future implementer can see what changed.

### Measured, 2026-08-31

| What | Value | Command |
|---|---|---|
| `TYPE_RESOURCE_FIELD_FORBIDDEN` | retired (reserved-not-emitted) | `sed -n 1008,1019p src/rules/table.rs` (2026-09-01) |
| plan-114 letters archived | 5 (A–E) | `ls planning/completed/plan-114-*` (2026-09-01) |
| `Picture.image` type | **`RES canvas::Image`** — plan-116-I landed | `grep -n 'RES canvas' src/codegen/builtins/canvas/mod.rs`; `ImageRef` no longer exists (2026-09-04) |
| `Text.font` type | **`RES canvas::Font`** | same grep; `FontRef` no longer exists (2026-09-04) |
| `ImageRef` / `FontRef` | **gone**, and their absence is pinned | `grep -n 'imageRef\|fontRef' src/codegen/builtins/canvas/mod.rs` → prose plus `mod.rs:1339`, a test that iterates `["imageRef", "fontRef"]` and asserts neither resolves (2026-09-04) |
| Close is a whole-word store | `store_u64(1, record, RESOURCE_OFFSET_CLOSED)` | `lower_destroy_image` in `func_destroy_image.rs`; no bitfield anywhere (**J7**) |
| Resources declared by `canvas` | 2 (`Image`, `Font`) | `grep -n 'pkg\.add_resource' src/codegen/builtins/canvas/mod.rs` → 2 (2026-09-04). **Anchor on `pkg.add_resource`, not `add_resource`**: the bare form greps 4, because the module comment and an inline comment both name it (**J3**). |
| `live_slots` on both | `&[]`, `sendable: false` | the `live_slots` and `sendable` fields of each `pkg.add_resource` call in `mod.rs` |

### Verified properties

- **The two `canvas` resources are not transfer-audited.** Read the two `pkg.add_resource` calls in `mod.rs`:
  `sendable: false`, `live_slots: &[]`, with the comment *"Not audited for transfer
  (bug-464 left canvas out of scope). Empty here is only consistent with
  `sendable: false`; opting an image in means auditing its record tail first, not just
  flipping the bit."* A group is worker-owned state that the graphics thread *reads*,
  so this letter must establish whether group ownership constitutes a transfer under
  plan-114's rules. **ANSWERED 2026-09-04 — no** (**J5**, and §4.1): the group stores a
  pointer, the resource record never moves arenas, and the graphics thread merely reads
  it, which is the pattern every published scene already uses. `live_slots` and
  `sendable` are unchanged. The record-tail audit the "if yes" branch called for is
  recorded in **J5** anyway, so a future `sendable: true` does not have to re-derive it.

- **A `Picture` holding a `RES Image` cannot cross a thread data plane, and after
  plan-114-A that is a hard compile error rather than a silent acceptance.**
  plan-114-A added **`2-203-0138 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`** — now landed
  on main (`grep -n TYPE_THREAD_RESOURCE_PLANE_REQUIRED src/rules/table.rs` → the rule
  is at `:741-746` with message *"a resource cannot cross the thread data plane"*;
  re-verified 2026-09-03). Combined with
  `canvas::Image`'s `sendable: false`, a record carrying one is refused at any thread
  boundary.

  **This is a design input, not a discovery to be made during implementation.** If any
  part of this letter — or of a program using it — expects a `Picture`, a group's item
  list, or a `List OF DrawItem` containing one to be sendable, that has to be designed
  in deliberately, which means auditing `Image`'s record tail and setting `live_slots`
  rather than flipping `sendable`. Raised by mfb-76, 2026-08-31.

- **The plane rule does not reach plan-116-G's group table, and the reason is
  specific.** Per mfb-76, `2-203-0138` fires **only** from `check_thread_sendability`
  and the `thread.*` call checks in `src/ir/verify/resources.rs` — that is, only where a
  `Thread`/`ThreadWorker` *type's* planes are declared, or an actual
  `thread::start`/`send`/`transfer`/`accept` argument type is checked. It is a rule
  about thread-boundary **types**; nothing in it inspects storage duration or which
  thread touches a value, so process-global storage, a module-level `MUT`, and any
  buffer the graphics thread reads are all outside its reach. plan-116-G's design
  stands on this axis.

  **Re-verified 2026-09-03 against `resources.rs` as merged, and it holds.**
  `grep -n emit_thread_resource_plane_required src/ir/verify/resources.rs` gives one
  definition (`:574`, whose own doc comment says *"the one place `2-203-0138` is
  worded"*) and exactly **three** callers, at `:559`, `:622` and `:749`, inside
  `require_thread_sendable` (`:555`), `check_thread_sendability` (`:591`) and
  `check_thread_boundary_sendability` (`:662`). All three take a `ParameterType` and
  nothing else; none can see storage duration or which thread touches a value. So the
  claim is measured rather than inherited, and Phase 1 can tick this row by re-running
  that grep instead of re-deriving the argument.

- **The constraint that *does* govern the group table is the arena one, not the rule.**
  Arena state is per-thread and a spawned thread sees its own zeroed copy, so
  cross-thread data needs a genuine process-global symbol and no thread may free
  another's block (`.ai/canvas-threading.md` §2–§3). That — not the plane rule — is what
  plan-116-G's Phase 1 audit must be driven by, and plan-116-G §4.1/§4.3 is already
  written against it.
- **`.ai/canvas-threading.md` §7's gate already defers the OS-side free** past any
  in-flight frame. So "close on group free" composes with it: the group's close sets
  the closed flag, and the existing gate frees the backing. This letter should add
  *no* new deferral mechanism — see §3.

## 3. Design Overview

*Rewritten 2026-09-04 (Phase 1). What it said: "three pieces, and the whole letter is
deliberately small because plan-116-G already built the hard part." Two of the three
turned out to be nothing, and a fourth piece — the one the Goal actually rests on — was
missing. **The letter is not small.***

1. ~~**Establish whether group ownership is a "transfer"**~~ — **answered, no** (**J5**,
   §4.1). The record never changes arena and the graphics thread only reads it, which is
   what it already does for every published scene. `live_slots` and `sendable` unchanged.
   Not a piece of work; a question with an answer.
2. ~~**`setGroup`'s deep copy takes the resources with it**~~ — **already true, and it is
   the problem rather than the solution.** `List OF DrawItem` is still
   `type_is_memcpy_copyable` (`flatness_walk`'s `Res(_)` arm), so the existing
   `copy_flat_block` copies the 8-byte pointer and the group gets an **alias** (**J4**
   ¶3, **J5**). There is nothing to route: at install time nothing new comes into being.
3. **The group's free path closes what it owns**, immediately before releasing the
   buffer. Real, and §4.3 places it at `emit_free_items_block` — the chokepoint **both**
   free sites funnel through, not just the gated one (**J4** ¶2). **No new deferral**:
   the close sets the closed flag and the existing texture gate
   (`.ai/canvas-threading.md` §7) does the rest.
4. **NEW — `setGroup` moves every resource transitively reachable from `items`.** This is
   what Goal bullet 1's *"the caller's bindings may go out of scope without closing
   them"* requires, and nothing implements it (**J9**). Without it Phase 2's own test
   fails regardless of the free path, because scope-drop closes the caller's `RES` and
   the group's alias does not stop it. It is also the only defence against §4.3.1's
   sharing hole. New analysis in `src/ir/verify/`; Phase 2's first box.

**Where the correctness risk concentrates:** double-close and use-after-close across
the worker/graphics boundary. plan-59-B's runtime backstop makes a second close a
defined `ErrResourceClosed` rather than corruption, which bounds the damage — but a
group closing an image a *scene* still names would make that scene draw nothing, which
is a silent wrong picture. ~~The rule that prevents it is already written:
`.ai/canvas-threading.md` §7 says a `Picture` carries a value handle, so *"presenting a
stale one draws nothing rather than raising"*~~ — **re-derived 2026-09-04, and §7 does
not prevent it; §7 is the mechanism by which it happens.** "Draws nothing rather than
raising" is what makes the wrong picture *silent*. The sentence survives plan-116-I in a
new mechanism (`imageHandle` answers `0`, `errors: vec![]`, closed read before handle),
and that changes nothing about this risk.

**And the risk is wider than a scene.** Two *groups* can name one image just as easily —
probed, and it compiles clean today (**J9**). §4.3.1 carries this paragraph's warning
into the design and the phases, which is what it never had: it was stated here as a risk
and then not addressed by any of §3's pieces or any phase box.

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

*Written 2026-09-04 against landed code, per Phase 1. The three constraints below the
line were fixed before plan-114 landed and still hold; everything above it is new.*

### 4.1 What ownership can and cannot mean here

The group holds an **alias**, not a copy. `emit_set_group`'s `copy_flat_block` copies the
items block, and a `Picture`/`Text` slot inside it is one 8-byte pointer to a resource
record that stays where the worker allocated it — `flatness_walk`'s `ParameterType::Res(_)`
arm, which is why `List OF DrawItem` is still `type_is_memcpy_copyable` after plan-116-I
(**J4**, **J5**).

So "the group takes ownership" cannot mean *the group has its own resource*. It can only
mean **the group becomes responsible for closing the one that exists**. Two consequences:

* There is nothing to do at install time. Nothing new comes into being, so there is no
  install-side step to write — which is why §4.3's "ownership attaches at the copy" is a
  place to put a *comment*, not code.
* The whole letter is the **free** path.

### 4.2 Closing is cheap and idempotent, which is what makes this tractable

`lower_destroy_image` is two instructions: `move_immediate(flag, 1)` and
`store_u64(flag, record, RESOURCE_OFFSET_CLOSED)`. **There is no runtime call.** The
OS-side free is already deferred behind the backend's own
`closed AND lastUsedFrame < lastCompletedFrame` gate (plan-98-D). `lower_destroy_font`
adds one step before the flag — `emit_unregister_font`, so a renderer never finds a
published block whose resource is closed.

Both write the flag **unconditionally**, so closing an already-closed resource is a
no-op. That matters more than it looks: it means the letter does not need to prove
"exactly once" at the instruction level. It needs to prove *at least once* and rely on
idempotence for the rest — including the case where the caller's own scope-drop cleanup
also closes.

`RESOURCE_OFFSET_CLOSED` (16) is a **plain boolean word**, not a bitfield: both
`lower_destroy_image` and the resource system's own `emit_closed_resource_record` store a
whole-word `1` over it, and no second bit is defined anywhere. So the close this letter
emits is a whole-word store too — a read-modify-write would be inventing a hazard that
does not exist (**J7**).

### 4.3 Where the close goes: one chokepoint, two callers

There are **two** free sites, and the obvious one is not the dangerous one (**J4**):

| site | when | gated? |
|---|---|---|
| `emit_group_reclaim` | `canvas::groupReclaim()`, first thing in `present` | yes — `RETIRED_ITEMS != 0` and `frame_now >= stamped` |
| `emit_retire_current_items` | a **second `setGroup` in one frame**, freeing the block retired by the first | **no** — deliberately bypasses the gate |

Both funnel through `emit_free_items_block`. **The walk-and-close goes there, immediately
before `emit_arena_free`** — after the `OWNED_BYTES` subtraction, so the accounting is
unchanged, and before the block is released, so the pointers are still readable.

Putting it in the helper rather than at the two call sites is not tidiness: a close added
only at the reclaim site leaves the second-`setGroup`-in-one-frame path silently
unclosed, and that path has no test today.

**The ungated site turns out not to need a new argument, and the reason is worth stating
exactly, because the obvious version of it is wrong.** plan-116-G's justification, in
`emit_retire_current_items`'s own doc comment, is *"the drain gate runs at the top of every
`present`, so a still-occupied retired word means no frame has completed since it was
retired, which means no render can have started reading it"* — an argument about **frames**,
not about publication. Adding a close there inherits it rather than needing a new one, and
by a margin: **the close is strictly weaker than the free that already happens on that
line.** Anything that could observe the close would first have to read the block, and the
free already asserts nothing can. Closing is also `store_u64(1, …)` with no OS-side effect
at all (§4.2) — §7's *"close never frees … safe at any instant, from the worker, with no
knowledge of what the graphics thread is doing."*

So the ungated site is safe **for the group's own reachability**. What it is not safe
against is §4.3.1.

#### 4.3.1 Two groups can name the same image, and closing is global

This is the hole in "the group owns its items", and it is not reachable from the free
path's own reasoning, because it is not about reachability at all.

`canvas::Image` is a handle. Nothing stops a program from building two `Picture`s from one
image and installing them in two different groups — or one in a group and one in the live
scene. The resource record is **shared**: there is one `closed` word, and a close from any
holder closes it for every holder. So a group that "owns" its items and closes them on free
does not free *its* image; it closes *the* image, and the other group's `Picture` silently
starts drawing nothing.

The free path cannot detect this. Its argument establishes that nobody is reading **this
block** — which is true, and irrelevant, because the other holder is reading a different
block that points at the same record.

**There is no refcount to reach for.** `.ai/canvas-threading.md` §7 is explicit — *"There
is no refcount, and there is nothing to count"* — and `CANVAS_GROUP_REFS` is written and
decremented but never read as a predicate (**J4**), so it is not a latent one either.
Introducing resource refcounting is a change to the RES model, far outside this letter, and
§Non-goals rules it out.

Three ways out, and Phase 2 must pick one **before** writing the walk, because the walk is
the same code in all three and only the surrounding contract differs:

1. **Ownership is the documented contract, and sharing is the program's error.** `setGroup`
   states that the group closes the images and fonts its items name, so naming one image in
   two groups is a use-after-close the program authored. Cheapest, and consistent with
   `destroyImage` moving its binding for a *direct* call — but §4.6 is exactly why that
   analogy does not carry: the compiler cannot see this one, so the failure is silent and
   at render time, which is the worst shape a diagnostic can have.
2. **`setGroup` moves every resource transitively reachable from its `items` argument**,
   the way `destroyImage` moves its one, making the second install `2-203-0055`. The only
   option where the compiler catches it.
3. **Drop ownership for shared-by-construction resources** and keep this letter to the
   case it can defend.

**Measured 2026-09-04 (J9), because two cheaper readings of option 2 are both dead.**

*Moving the list is not enough.* The two installs pass **different** lists (`[a]` and
`[b]`); the binding used twice is `img`, inside two separate `Picture` constructions. A
move attached to the `items` argument moves a list, and catches nothing.

*Moving at the constructor is impossible.* One could make `Picture[image := img]` consume
`img` — but `canvas::present(items AS List OF canvas::DrawItem)` and
`canvas::setGroup(name AS String, items AS List OF canvas::DrawItem)` take the **identical
type**, so a `DrawItem` cannot know at construction which it is destined for, and §Non-goals
requires that `present` not own. Probed: a `FOR` loop building a fresh `Picture` from one
long-lived image and presenting it each pass compiles today, and it is what every canvas
program does. A consuming constructor would refuse it on the second frame.

So option 2 survives only in its transitive form: the move is decided **at the `setGroup`
call**, and the checker must walk from the argument through the list to the records to the
`RES` slots inside them. Nothing does that today — the two-group probe compiles clean with
no diagnostic. Whether the move analysis in `src/ir/verify/` can be extended to it is
implementation work, and it is Phase 2's first box.

**This is a Phase 2 decision with a Phase 3 consequence, and it is recorded as an Open
Decision rather than settled here.**

### 4.4 The walk this letter has to write

**Nothing walks a group's stored items today.** Every existing walk goes through
`canvas::groupItems(slot)`, which returns a *copy*, and runs on the **graphics thread** —
`__canvas_appendDraw`, `__canvas_groupSignature`, `__canvas_memoGroup`,
`__canvas_drawGroup`. None of them visits `Picture.image` or `Text.font`; the only code
that reads a resource out of an item is `helper_geometry.rs`'s six `canvas::fontHandle`
sites (**J4**).

So the walk is new, and it must run on the **worker**, because the close writes to a
record in the worker's arena and `emit_free_items_block` already runs there.

**The existing owned-container machinery does not fit.** An owned list carries **one**
`OwnedListDrop`, and `builder_resource_cleanup.rs` explicitly refuses *"a record with two
`RES` fields of differing resource types"*. A `DrawItem` list holds an `Image` (via
`Picture`) and a `Font` (via `Text`) — two close ops. `emit_owned_list_drain` cannot be
pointed at it unchanged, and widening that primitive to carry a close op *per field* is a
change to shared cleanup code that every other package depends on.

~~The narrower option … steps the items block by `ITEM_BLOCK_SIZE`, switches on the kind
word …~~ **Wrong, and corrected 2026-09-04 before it was built (J13).**
`ITEM_BLOCK_SIZE = 208` is the **GPU quad record** — the per-instance `ItemBlock` the
Vulkan and Metal shaders index, whose stride is pinned against glslang's std430
reflection by `the_item_block_matches_the_std430_stride`. It has nothing to do with what a
group stores. A group's buffer is an ordinary MFB collection block of
`List OF canvas::DrawItem`: `emit_set_group` produces it with `copy_flat_block`, and
`emit_free_items_block` sizes it with
`emit_inlined_block_size_from_ptr_slot(list_of(named("DrawItem")), …)`. Its element is a
**union value**, so the walk steps by the union's size and reads the union **tag** — there
is no "kind word" at offset 64 to switch on.

**So the shape of the walk is an open Phase 3 decision, not a detail.** Two candidates:

1. **A Rust walk in `gen_group.rs`.** Keeps the change beside the free it hangs off, but
   means open-coding the `DrawItem` union's layout — tag, payload offset, and each
   variant's `RES` field offset — in codegen. Every one of those is a constant that
   already exists somewhere else, and a second copy of a layout is the shape that
   miscompiles when a variant is added.
2. **An MFBASIC helper called from the worker.** `MATCH` over the items is exactly the
   operation, the compiler computes every offset, and adding a variant that carries a
   resource is then a compile error in the helper rather than a silent miss. `canvas::`
   already has six such walks (`__canvas_appendDraw`, `__canvas_groupSignature`,
   `__canvas_memoGroup`, `__canvas_drawGroup`, …). The obstacle is reach: they all go
   through `canvas::groupItems(slot)`, which returns the **live** buffer, and this walk
   needs the **retired** one.

**Resolved: option 2 — the walk is an MFBASIC `MATCH`, driven from `#canvas_present`.**

*(First formulation, discarded: "change `groupReclaim()`'s return type to
`List OF canvas::DrawItem` — the items of every buffer it just freed." One member changed,
no new ones, very tidy — and it requires **concatenating N collection blocks inside Rust
codegen**, which is the one part of this there is no existing helper for. `copy_flat_block`
copies one block; nothing appends. Discarded before it was built rather than after.)*

**The gate stays in Rust; only the walk moves to MFBASIC.** `groupReclaim`'s scan splits
into a finder and a freer, both internal-only:

* `canvas::nextReclaimableGroup() AS Integer` — the first slot whose retired buffer the
  gate has opened for, or `-1`. **Frees nothing.** This is `emit_group_reclaim`'s existing
  loop with the two free calls replaced by a `RETURN i`.
* `canvas::retiredItems(slot) AS List OF canvas::DrawItem` — `emit_group_items` with
  `CANVAS_GROUP_RETIRED_ITEMS` in place of `CANVAS_GROUP_ITEMS`. A copy, for the same
  reason.
* `canvas::reclaimGroupSlot(slot)` — what `groupReclaim`'s body already does for one slot:
  free the items block, free the retired name, zero both words. **Unconditional**; the
  caller has just been told this slot is due.

`#canvas_present` then replaces its single `canvas::groupReclaim()` with

```basic
LET slot AS Integer = canvas::nextReclaimableGroup()
WHILE slot >= 0
  FOR EACH gone IN canvas::retiredItems(slot)
    MATCH gone
      CASE Picture(p)
        canvas::destroyImage(p.image)
      CASE Text(t)
        canvas::destroyFont(t.font)
      CASE ELSE
    END MATCH
  NEXT
  canvas::reclaimGroupSlot(slot)
  slot = canvas::nextReclaimableGroup()
WEND
```

**Why not the obvious `FOR i = 0 TO CANVAS_MAX_GROUPS - 1` in MFBASIC.**
`CANVAS_MAX_GROUPS` is **256**. A per-present MFBASIC loop calling a builtin per slot
would put 256 calls on the per-present path — the exact axis plan-116-G optimised, whose
`groupItems` doc comment says the copy is *"charged to the frame that draws, not to
`present`"*. The finder keeps the scan in Rust, so **a present with nothing due costs one
call and one scan, exactly as today**, and the loop above executes zero times.

**No re-evaluation hazard between finder and freer.** `frame_now` only advances and both
run on the worker with nothing between them, so a slot the finder reported as due cannot
have become undue by the time `reclaimGroupSlot` runs.

Four properties this buys, none of which the other shapes have:

* **No new *public* surface.** All three members are `internal_only: true`, so none is
  reachable from a program or rendered by `mfb man` — §Non-goals rules out new members a
  user can call, and these are the same category as `groupItems`, `groupResolve` and
  `groupRevision` that plan-116-G already added. `groupReclaim` is not deleted so much as
  renamed and narrowed to one slot.
* **One gate evaluation per slot.** The finder evaluates it; the freer does not
  re-evaluate it. A design where both tested the gate could disagree if a frame completed
  between them — here only one of the two tests it at all.
* **The layout is the compiler's problem.** A `MATCH` that a new `DrawItem` variant must
  handle is a compile error; an open-coded tag offset that a new variant must not break is
  a hope (**J13**).
* **The handles are readable, and §4.3's ordering constraint is met directly.** The
  close runs **before** `reclaimGroupSlot` frees anything, which is what §4.3 asked for.
  (It would also have been safe after: `retiredItems` returns a **copy**, and the resource
  *records* are separate arena allocations that freeing the items block does not touch. But
  the loop above does not need that argument, so it does not rest on it.)

Two things Phase 3 must check rather than assume:

1. ~~**`canvas::destroyImage(p.image)` must be legal**~~ — **checked 2026-09-04: it is.**
   A `MATCH` over a `canvas::DrawItem` whose `CASE canvas::Picture(p)` arm calls
   `canvas::destroyImage(p.image)` compiles clean, so a `RES` read straight out of a
   record field may be passed to a consuming (non-`RES`) parameter. plan-114-C's
   `value_aliases_live_resource` covers the adjacent `RES g = h.handle` shape; this one
   needs no intermediate binding.

   *(Noted in passing, because it will bite whoever writes the probe rather than the
   helper: from a **user** program the variant must be written `CASE canvas::Picture(p)`.
   Unqualified `CASE Picture(p)` is `2-201-0015 SYMBOL_UNKNOWN_TYPE`. Inside the package's
   own injected source — which is where this helper goes — the unqualified form is the one
   that works, as `helper_render.rs`'s `CASE Group(g)` shows.)*
2. **`retiredItems`' copy must register no cleanup of its own**, or `#canvas_present`'s
   scope exit closes the resources a second time — harmless by idempotence (§4.2), but it
   would also mean the *live* scene's `groupItems` copies do the same, which is not
   harmless. **Measured 2026-09-04: it registers none.** A named
   `LET items AS List OF canvas::DrawItem` local, built in a `SUB` that presents it and
   returns, leaves the font open — `canvas::measureText` afterwards prints
   `STILL-OPEN width=32.00` rather than raising. `is_resource_owning_container` is false
   for it: not a `List OF RES T`, and `record_res_field_types` has no entry for a union.
   Same reason `groupItems`' copies register none today, now measured in the shape that
   matters — a **named local**, not a temporary.

The close itself remains two instructions per resource with no call and no tag to check
(§4.2).

### 4.6 The compile-time refusal does not reach this letter's close

`.ai/canvas-threading.md` §7 and row R3 say a closed image cannot be named again because
`destroyImage` **consumes its binding**. Probed, and true for a *direct* call — a program
that calls `canvas::destroyImage(img)` and then builds a `Picture` from `img` is refused
with `2-203-0055 TYPE_USE_AFTER_MOVE`, *"Binding `img` was moved and cannot be used
again"*. The mechanism is the parameter type: `destroyImage`'s `image` parameter is
`ParameterType::named(IMAGE_TYPE_ID)`, not `res(...)`, so passing a resource to it is a
move.

**It does not generalise, and the exception is exactly this letter's shape.** Close the
image through a helper that takes `RES img AS canvas::Image` and the caller's binding is
*not* moved — `cli_canvas_image_resource.rs`'s `closedRefuses` calls `getSize` on it
afterwards and pins the **runtime** `ErrResourceClosed`. A `RES` parameter is an alias, so
nothing is consumed at the call site.

A group closing its own items is that case and not the first one: the close happens inside
the runtime, against a record the program still has a live binding to. So this letter
**cannot** lean on the compile-time refusal, and the properties it has to preserve are the
runtime ones — close is idempotent (§4.2), and a render reads `closed` before the handle
and draws nothing (`func_handle_bridge.rs`, `errors: vec![]`).

### 4.5 The slot is full

`error_constants.rs` says so already: *"Both spare words are now used: `RETIRED_ITEMS` and
`RETIRED_NAME`. plan-116-J will need to grow the slot to 128 (still a power of two, still
a shift) rather than find room here."* If this letter needs a per-slot word — an owned
flag, say — growing `CANVAS_GROUP_SLOT_BYTES` to 128 and `SHIFT` to 7 is the sanctioned
move, and the new word must also be added to
`the_group_slot_size_is_a_power_of_two_matching_its_shift`, which takes the max over a
hardcoded list.

**Whether a new word is needed at all is open.** If every group owns its items
unconditionally, no flag is required and the slot stays at 64.

---

Fixed before plan-114 landed, and still true:

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
- [x] Read plan-114-B, -C and -D **as landed** and write §4's detailed design against
      them. §4 is now §4.1–§4.6, written against `gen_group.rs`, `func_destroy_image.rs`,
      `func_handle_bridge.rs`, `builder_resource_cleanup.rs` and `flatness_walk` as
      landed. Its load-bearing findings are **J4** (two free sites, one chokepoint; no
      `refs == 0` term; the slot is full), **J5** (alias, not copy) and **J7** (the close
      is a whole-word store, and the compile-time refusal does not reach this letter).
- [ ] Settle the §2 open question: does installing a resource into a process-global,
      graphics-thread-readable group buffer constitute a transfer under plan-114's
      rules? If yes, audit `Image`'s and `Font`'s record tails and set `live_slots`
      accordingly (the `live_slots` field of each `pkg.add_resource` call in `mod.rs`) — *"opting an image in means auditing its
      record tail first, not just flipping the bit."*
      **Answered: no** (**J5**, §4.1). The group stores a pointer; the resource record
      never changes arena; the graphics thread only reads it, which is what every
      published scene already does. `live_slots` and `sendable` stay as they are. The
      record-tail audit the "if yes" branch asked for is recorded in **J5** regardless,
      so a future `sendable: true` need not re-derive it.
- [x] Read `.ai/canvas-threading.md` §7 **as plan-116-I rewrote it** (a `Picture`
      now carries a `RES`, and the renderer reads a closed handle as the zero id, so
      "draws nothing rather than raising" survives in a new mechanism). Verify the
      rewritten paragraph against the landed code rather than against this plan.
      Four claims checked, three verified as written and one corrected in the doc
      (**J7**): the move refusal is real but holds only for a *direct* `destroyImage`, so
      §7 gained the qualifier and the matrix gained **R3b**. Verified unchanged:
      `imageHandle`/`fontHandle` carry `errors: vec![]` and cannot raise; the closed
      guard is emitted **before** the handle load in `lower_handle`; `imageRef`/`fontRef`
      survive only as prose and as `mod.rs:1339`, a test asserting neither resolves.

Acceptance: §4 of this document is written against landed code, §2's table is current,
and the transfer question has a recorded answer with the audit behind it.
Commit: —

### Phase 2 — Ownership on the way in

- [x] **First: settle §4.3.1 by measurement.** Measured 2026-09-04 (**J9**, **J10**).
      Probes: the two-group program compiles clean, so the hole is real; the per-frame
      `present` loop compiles, so a consuming *constructor* is impossible; and
      `destroyImage(img)` followed by `present([a])` compiles, so containment must not be
      modelled as aliasing. **Option 2 is reachable in its transitive form and is the
      choice** — `check_resource_moves` already carries plan-59-E's alias graph and
      closure, and what is missing is a second, *directed* `contains` relation plus a
      consuming-parameter flag. Sites in **J10**.
- [x] Implement the transitive move: the `contains` relation in
      `check_resource_moves` (`src/ir/verify/resources.rs`), a new `consumed_contained`
      beside `consumed_resource` (`src/ir/verify/link.rs`) returning a set, and the
      consuming-parameter flag on `setGroup`'s `items`. **Containment is directed**:
      consuming the container moves what it contains; consuming a contained resource
      must leave the container usable, or the documented *"closing it while a scene
      still names it draws nothing rather than failing"* becomes a compile error.
      Landed in three parts — the registry seam
      (`pkg.add_consuming_parameter` + `builtin_consuming_parameter_index`), the
      verifier's directed `contains` relation, and codegen's
      `deactivate_consumed_cleanups`. **J12** records what the verifier half alone did
      NOT do, and the three wrong guesses about where an `abi_function` call site
      arrives.
- [x] Correct §Non-goals' *"`setGroup`'s signature is unchanged"* (**J10**): the rendered
      signature is unchanged but its meaning is not, and that belongs on the man page.
      Done — the Non-goal now reads *"no new members, and no change to any member's
      rendered signature"*, with the reason spelled out. Phase 4 owns the man-page half.
      The `present` Non-goal beside it also had its two citations corrected (**J4**:
      neither string exists any more) and is now **enforced by a test** rather than by
      prose, `set_group_items_is_the_consuming_parameter` asserting
      `builtin_consuming_parameter_index("canvas.present")` is `None`.
- [x] ~~`setGroup`'s deep copy routes resource ownership per Phase 1's design instead of
      copying a handle.~~ **Moot, with the evidence §4.1 predicted.** The copy already
      produces an alias — `List OF DrawItem` is still `type_is_memcpy_copyable` because
      `flatness_walk`'s `Res(_)` arm returns true — so nothing new comes into being at
      install time and there is no ownership to route through `copy_flat_block`. The
      work this box was reaching for turned out to be the *deactivation* on the caller's
      side, which is the second half of **J12** and is done. Deliberately not padded
      with an install-side step to make the phase look substantial.
- [x] Tests: a program that opens a **font**, `setGroup`s a `Text` naming it, drops its
      own binding, and presents the group — **the glyphs still draw.**
      `tests/rt_canvas_group_ownership.rs`, four tests, and the shape matters more than
      the count: `glyphs=0` is equally consistent with *"the font was closed too early"*
      and *"group text never renders"*, so the file ships
      `the_control_draws_with_the_binding_alive` alongside, and it is what makes the
      ownership assertion mean anything. **Proven RED**: with
      `deactivate_consumed_cleanups` commented out, the two ownership tests fail
      (`glyphs=0`) and the control stays green (`glyphs=1`). *(Was "the image still
      draws". **J11**: measured, and the identical program with the binding still alive
      renders an all-black frame too, because `helper_geometry.rs` gives `Picture` the
      `NONE` geometry kind and `canvas::imageHandle` has no caller in any renderer. A
      pixel assertion here would fail for a reason that has nothing to do with this
      letter, and would keep failing after the letter was correct.)*
- [x] Tests: the §4.3.1 case itself — one image, two groups — pinned to whatever
      §4.3.1 resolves to. **Option 2**, so a `tests/syntax/` fixture:
      `tests/syntax/resources/canvas-setgroup-consumes-items`, whose golden is the
      `2-203-0055` on the second `Picture`. Green through `scripts/test-accept.sh`.

Acceptance: §4.3.1 has a resolution recorded in Open Decisions **with the probe output
behind it**, and a test pinning that resolution; **the drop-the-binding case leaves the
resource OPEN** — asserted on what the group draws through a live resource rather than on
a `Picture`'s pixels, because no `Picture` draws an image on any backend yet and a
`Picture` assertion cannot tell *"owned correctly"* from *"nothing draws"* (**J11**);
`cargo test --no-fail-fast` green; every canvas golden byte-identical.

**MET on macOS.**

| gate | result |
|---|---|
| §4.3.1 resolved with probe output | **J9**/**J10**, three probes; Open Decisions records option 2 transitive |
| test pinning the resolution | `tests/syntax/resources/canvas-setgroup-consumes-items`, green through `scripts/test-accept.sh` |
| the drop-the-binding case | `tests/rt_canvas_group_ownership.rs`, 4 passed — and **proven RED** with the fix disabled |
| `cargo test --release --no-fail-fast` | `rc=0`, **114 targets ok, 0 FAILED** |
| `scripts/artifact-gate.sh target/release/mfb all` | 1358 tests, 1521 builds, **1878 goldens, 0 diffs** |

*(1358 rather than plan-116-I's 1357: the new syntax fixture. The `retiredItems`/
`nextReclaimableGroup` work is Phase 3 and is not in these numbers.)*
Commit: `321dfddaf` (boxes), `1d2f1ff3e` (codegen), `49257cfba` (verifier), `1fbddbf09` (registry)

### Phase 3 — Ownership on the way out (largest blast radius)

- [x] The group free path closes each owned resource once, on the worker, before
      releasing the buffer. Per §4.4: `emit_group_reclaim` split into
      `canvas::nextReclaimableGroup()` (finder, frees nothing) and
      `canvas::groupReclaim(slot)` (unconditional one-slot freer — **the name is kept**,
      which is why only two rows were added to each table rather than three), with
      `canvas::retiredItems(slot)` between them; `__canvas_closeRetired` drives the
      `MATCH` from `#canvas_present`. `emit_group_items` and `emit_retired_items` share
      one `emit_items_at`, so the "out-of-range and empty read identically" rule has one
      home. Five tables updated: `macos_aarch64/mod.rs`, `linux_common/mod.rs`,
      `win_x86_64/mod.rs`, `data_objects.rs`, `module_analysis.rs`.
- [x] Verify §4.4's open check 2: `retiredItems`' copy registers no cleanup of its own.
      **Measured 2026-09-04, and it does not.** A `SUB draw(RES face AS canvas::Font)`
      that builds a named `LET items AS List OF canvas::DrawItem = [tag]` naming the font,
      presents it, and returns — then `canvas::measureText(face, …)` in `main` afterwards.
      Prints `STILL-OPEN width=32.00`, so the list's scope exit closed nothing. Confirms
      `is_resource_owning_container(List OF DrawItem)` is false in the shape that matters
      (a *named local*, not just a temporary), which is why `groupItems`' copies register
      none today. Worth measuring rather than assuming: if it had been true, the **live**
      scene's copies would have been closing resources on every frame.
- [x] `setGroup` replacing a live group closes the old buffer's resources **that the new
      buffer does not also name** (**J14**). `__canvas_closeRetired` compares each retired
      item's `canvas::imageHandle`/`fontHandle` against the live buffer's, skipping `0` on
      both sides. Measured both directions: 6 rebuild iterations keep `glyphs=1` and
      `groupBytes` flat, and a replacement that drops the image prints
      `OPEN-AFTER-INSTALL` then `CLOSED-BY-THE-GROUP`.
- [ ] Decide and pin the group-and-live-scene case (**J14**): `present` does not consume,
      so a `Picture` built before a `setGroup` can reach the scene, and the group's free
      would close it out from under the scene. Either extend the "no live buffer names it"
      scan to the published scene, or state the limitation **with a test pinning the
      observable outcome**. Leaving it implicit is how §3's risk paragraph became **J8**.
- [ ] Tests, extending plan-116-G Phase 5's race matrix — add the rows to
      `.ai/canvas-threading.md` §8 as well:
      - group owning an image → `removeGroup` → graphics mid-frame: the frame completes
        and the image is **still open** during it. *(Was "still samples the texture" —
        there is no texture; **J11**.)*
      - the same, then a completed frame: the image closes exactly once. Observable as
        `canvas::getSize` raising `ErrResourceClosed` where it did not before.
      - a group owning an image, and a *scene* also drawing that image: §4.3.1's
        resolution decides the outcome; assert it, and assert it is not a crash.
      - `setGroup` replacing a group: the old resources close, the new ones do not.
      - 200 × install/remove of a group owning a `Font` and an `Image`: `groupBytes=`
        returns to baseline. *(The fd half is **vacuous today** and must not be reported
        as a pass: `createImage` allocates nothing outside MFB's own resource record, so
        there is no descriptor to grow — **J11**. A `Font` loaded from a file is the one
        that can hold one, so the `Font` half of this row is the real check.)*

Acceptance: all five rows pass; the 200-cycle loop shows no `groupBytes=` growth, and no
fd growth for the `Font` (`lsof` on the process, or the platform equivalent) — **the image
half of the fd check is vacuous until the sampler lands and must be recorded as such
rather than counted as a pass** (**J11**);
`cargo test --no-fail-fast` green on **mac RELEASE, mac DEBUG (`--bin mfb`) and box 2228 RELEASE** (plan-116-E **E6**: CI is `--release` on all five platforms, so the `debug_assert!`s run nowhere in it and the debug row has to be run here).
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
- [ ] `scripts/regen-ncodesum.sh`. Expect **0 diffs, and do not read that as
      evidence** — no `canvas` fixture is hashed (plan-116-F **F11**).

Acceptance: `cargo test --no-fail-fast` green on **mac RELEASE, mac DEBUG (`--bin mfb`) and box 2228 RELEASE** (plan-116-E **E6**: CI is `--release` on all five platforms, so the `debug_assert!`s run nowhere in it and the debug row has to be run here), `scripts/test-accept.sh`
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

- **~~`Picture` must migrate from `ImageRef` to `RES Image`, and it is nobody's job
  yet.~~ RESOLVED (2026-09-01):** the recommended option was taken, by user
  direction — the migration is its own letter, **plan-116-I**, between plan-116-H
  and this one, and this letter's prerequisites gate on it. This letter stays
  medium, written on the assumption the migration is not in it — which is now a
  guarantee rather than an assumption.
- **~~Is a group-owned resource a "transfer"?~~ RESOLVED (2026-09-04, Phase 1): no.**
  The recommendation here was to assume **yes**, and it was wrong — the shape `sendable`
  governs is a resource record **changing arena**, which a `thread::transfer` does and an
  install does not. The group stores a pointer to a block whose `Picture` slot stores a
  pointer to a record that never moves, and the graphics thread only reads it, exactly as
  it already reads every published scene. `live_slots` and `sendable` are unchanged.
  Audit and reasoning in **J5**; consequence for the design in §4.1.
- **~~What happens when a scene draws an image a group owns and the group is removed?~~
  RESOLVED (2026-09-04, Phase 1): the item draws nothing, and it is now the *only*
  possible outcome rather than the preferred one.** The recommendation was to preserve
  today's observable behaviour, and the mechanism plan-116-I left behind enforces it:
  the renderer reads the backend id through `canvas::imageHandle`/`fontHandle`, whose
  descriptors carry `errors: vec![]` — they **cannot** raise — and which return `0` for a
  closed resource, `0` already meaning "no such object". Verified in
  `func_handle_bridge.rs`: `emit_closed_guard` runs before the handle load, so the answer
  cannot be a stale non-zero id from a concurrent destroy.
- **NEW, and the one this letter now turns on — what happens when two groups name the
  same image?** Raised by §4.3.1, 2026-09-04. Closing is **global**: one `closed` word per
  record, so a group closing "its" image closes it for every other holder, which then
  silently draws nothing. The free path's safety argument cannot see this, because it
  reasons about who can read *this block* and the other holder reads a different one.
  There is no refcount and §Non-goals rules out adding one. **Recommend option 2 of
  §4.3.1** — `setGroup` moves the resource out of the caller's binding, the way a direct
  `destroyImage` does — because it is the only one of the three that turns a silent
  render-time wrong picture into a compile error, and this letter exists because
  plan-116-I made these resources visible to the type system in the first place. Settling
  it needs a measurement (whether a `List OF DrawItem` built inline reaches `setGroup` as
  something the move checker can attribute to a binding), so it is **Phase 2's first
  task**, not a document decision.
  **RESOLVED 2026-09-04 — option 2, in its transitive form** (**J9**, **J10**). The two
  cheap readings are dead: moving the list catches nothing (the two installs pass
  different lists), and a consuming constructor is impossible (`present` and `setGroup`
  take the identical `List OF canvas::DrawItem`, so a `DrawItem` cannot know its
  destination, and the ordinary per-frame `present` loop would be refused on its second
  pass). What remains is a move decided at the `setGroup` call, walking argument → list →
  record → `RES` slot. `check_resource_moves` already has plan-59-E's alias graph and
  `alias_closure`; what is missing is a second, **directed** `contains` relation —
  directed because `destroyImage(img)` followed by `present([a])` compiles today and must,
  since `Picture.image` promises *"closing it while a scene still names it draws nothing
  rather than failing."*

## Corrections

**J14 (2026-09-04, Phase 3 preparation — found by probing the ordinary case) — Phase 3's
close, as §4.3 specifies it, breaks the commonest canvas program there is.**

**The pattern.** One long-lived font, a group rebuilt each frame:

```basic
RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e) … END TRAP
FOR i = 1 TO 3
  LET tag AS canvas::DrawItem = canvas::Text[…, font := face, …]
  canvas::setGroup("panel", [tag])
  canvas::present([canvas::Group[name := "panel", …]])
NEXT
```

**It compiles** — checked. Phase 2's move does not reject it, because
`check_resource_moves` is *"conservative straight-line dataflow"* by design: a loop body is
analysed once, and within that one pass `face` is read (building `tag`) **before** the
consume. A use-after-move across iterations is a deliberate false negative, and the doc
comment says why — *"so no valid program is ever rejected"*.

**And under Phase 3 it would be wrong at run time.** Iteration 2's `setGroup` retires
iteration 1's buffer. A frame completes; the gate opens; the free path closes every
resource that buffer named — including `face`, which **iteration 2's live buffer also
names**. The font closes while the group that is on screen is still drawing with it, and
`fontHandle` then answers `0`, so the text silently disappears. This is §4.3.1's sharing
hazard, arrived at not through an exotic two-group program but through the shape every
real canvas program has.

**So "the group closes what its buffer named" is not a correct rule**, and §4.3's
free-path design needs a discriminator. Three candidates, and only the third survives:

1. *Close only when the slot is emptied by `removeGroup`, never on replacement.* Fixes the
   rebuild pattern exactly and **leaks**: replacing a group whose items name font A with
   items naming font B never closes A.
2. *Refcount.* Ruled out by §Non-goals and by `.ai/canvas-threading.md` §7 — *"there is no
   refcount, and there is nothing to count"*.
3. **Close a retired resource only if no *live* buffer names it.** Bounded — the retired
   buffer's items against the replacing slot's items, both small — and it degenerates to
   the right answer in both directions: the rebuild pattern closes nothing, and a genuine
   replacement closes exactly what was dropped.

**Identity is comparable, and plan-116-I is what made it so.** Two aliases of one resource
cannot be compared as `RES` values, but `canvas::imageHandle`/`canvas::fontHandle` return
the backend id as an `Integer`, and they were added by plan-116-I as the replacement for
`imageRef`/`fontRef`. So the discriminator is an integer comparison in the same MFBASIC
`MATCH` helper §4.4 already calls for. That is a second use for a bridge that
otherwise has exactly one live caller (`fontHandle`, six sites in `helper_geometry.rs`) —
and it is why the closed-flag-before-handle read order that `func_handle_bridge.rs`
documents matters here too: comparing a *stale* non-zero id would keep a resource alive
that nothing names.

**One case this rule still does not cover, stated rather than hidden.** A resource named
by a group *and* by the **live scene** — `present` does not consume (§Non-goals), so a
`Picture` built before the `setGroup` can reach the scene without the move checker
objecting, and the group's free would then close it out from under the scene. Phase 3 must
either extend the "no live buffer names it" scan to the published scene or state the
limitation with a test pinning the observable outcome. **It must not be left implicit** —
that is exactly how §3's original risk paragraph became **J8**.

**J13 (2026-09-04, Phase 3 preparation) — §4.4's walk was specified against the wrong
block, and the constant it named belongs to the GPU.**

§4.4, as I wrote it earlier today, said the free-path walk *"steps the items block by
`ITEM_BLOCK_SIZE`, switches on the kind word"*. Both halves are wrong, and they are wrong
about the same thing: **`ITEM_BLOCK_SIZE = 208` is not the group's block.** It is the
per-instance GPU quad record — the `ItemBlock` the Vulkan and Metal shaders index, whose
208-byte stride is pinned against glslang's std430 reflection
(`the_item_block_matches_the_std430_stride`), and whose `ITEM_OFFSET_MISC = 64` is the
"kind word" I reached for.

What a group actually stores is an ordinary MFB collection block of
`List OF canvas::DrawItem`. `emit_set_group` builds it with `copy_flat_block`, and
`emit_free_items_block` sizes it with
`emit_inlined_block_size_from_ptr_slot(list_of(named("DrawItem")), …)` — the plain
collection path. Its elements are **union values**: the walk steps by the union's size and
switches on the union **tag**.

**Why this matters more than a wrong constant.** Written as specified, the walk would have
read 208-byte strides across a block whose elements are a different size, and switched on
a word that is not a tag — reading arbitrary bytes as resource pointers and storing a
closed flag through them. That is a wild write on the free path, in a subsystem where the
same code runs on a worker thread while a graphics thread reads nearby memory. It would
not have failed at the first fixture; it would have failed somewhere else, later.

Caught by reading `ITEM_BLOCK_SIZE`'s own doc comment before using it, which is the whole
lesson: the constant is well-named for its real job and badly named for the one I assumed,
and nothing about `items block` in the plan's prose distinguishes the two.

§4.4 now records the correction and turns the walk's shape into an explicit Phase 3
decision, recommending an MFBASIC `MATCH` helper over an open-coded Rust layout — a
`MATCH` that a new `DrawItem` variant must handle is a guarantee; a hand-written offset
that a new variant must not break is a hope.

**J12 (2026-09-04, Phase 2 — implemented) — the transitive move needed a second half the
letter never mentions, and finding where to put it took three wrong guesses.**

**The half that is obvious.** `ir::verify::check_resource_moves` gained a directed
`contains` relation beside plan-59-E's alias graph, `consumed_contained` in
`ir::verify::link` resolves a consuming parameter's argument through it, and
`pkg.add_consuming_parameter("setGroup", "items")` is the registry data behind
`builtin_consuming_parameter_index`. With that alone the §4.3.1 program is refused:

```
error[2-203-0055 TYPE_USE_AFTER_MOVE]: binding is used after move
              Binding `img` was moved and cannot be used again.
```

**The half that is not, and which the letter's §3 would have let a reader skip.**
`moved` is a **verification** set. Codegen emits its scope-drop closes from the cleanup
list and never consults it. So after all of the above, `install()` still closed the font
at its scope exit, the group went on naming a closed resource, and its `Text` drew
**zero glyphs** — silently, because a second close is a defined no-op, so nothing
crashed and nothing raised. Goal bullet 1's *"the caller's bindings may go out of scope
without closing them"* is a **codegen** statement, and satisfying the verifier does not
satisfy it.

`deactivate_consumed_cleanups` is the second half, modelled on `RETURN`'s deactivation in
`builder_exits.rs` because it is the same problem: a value whose ownership leaves this
scope must not also be closed by it. It needs a `resource_containment` pre-pass, because
the argument names a **container** — `setGroup("held", [tag])` — and the obligation to
drop belongs to `face`, whose name appears nowhere in the argument. Without the
expansion the walk finds only `tag`, whose type is a *union* and therefore not a
resource-owning container (`record_res_field_types` has no entry for a union), and
deactivates nothing.

**Three wrong guesses about where a `Body::abi_function` call site arrives, recorded
because none of them is visible from the code:**

1. Not `lower_value`'s `NirValue::Call` arm. A trace printed every target it saw and
   never `canvas.setGroup`.
2. Not its `CallResult` arm either, though `setGroup` *is* fallible
   (`ErrWrongMode`/`ErrCanvasGroupLimit`/`ErrOutOfMemory`).
3. Not `emit_call`, the apparent chokepoint: **798 lowered calls** in a program that
   calls `setGroup`, and not one of them named it.

It is **`NirValue::RuntimeCall`**, a NIR node of its own. `mfb build --nir` is what
settled it — the dump shows
`{"kind": "runtimeCall", "helper": "canvas", "target": "canvas.setGroup"}`. The same dump
also answered a question I would otherwise have guessed at:
`"resourceOwners": [{"name": "face", "owner": {"kind": "local"}}]`, so the font's
obligation is a plain `ActiveCleanup::Resource`, not an owned-list drain.

**Measured, with the control that makes it mean something.** `install()` opens a font,
builds a `Text`, `setGroup`s it and returns, dropping every binding; `main` presents a
`Group` naming it:

| | `glyphs=` | frame |
|---|---|---|
| before | `0` | entirely `(0,0,0,255)` |
| after | `1` (`glyphBytes=506`) | 646 pink pixels |
| control (binding stays alive) | `1` (`glyphBytes=506`) | 646 pink pixels |

The control is not decoration. `glyphs=0` is equally consistent with *"the font was
closed too early"* and *"group text never renders"* — which is exactly the trap the
`Picture` version of this test falls into (**J11**), and why
`tests/rt_canvas_group_ownership.rs` ships
`the_control_draws_with_the_binding_alive` beside the two ownership tests. Proven RED:
with `deactivate_consumed_cleanups` commented out, both ownership tests fail and the
control stays green.

**One thing to watch, written down because nothing enforces it.** The containment walk
now exists **twice** — `held_resources` over `IrValue` in `ir::verify::link`, and
`collect_consumed_locals` over `NirValue` in `builder_resource_cleanup`. They are two
lists: the types are distinct, so an arm added to one and not the other compiles. The
failure modes differ, which is worth knowing before choosing which to fix first — a
missing arm in the **verifier's** walk silently allows a use-after-close; a missing arm
in **codegen's** silently leaks, because the scope closes something the callee has taken
over and the callee's own later close is a no-op. Both walks carry a comment pointing at
the other.

**J11 (2026-09-04, Phase 2 — measured) — `canvas::Picture` does not draw an image on any
backend, so two of this letter's acceptance criteria cannot discriminate anything, and one
of its Goal bullets has nothing to observe. This is a cross-plan precondition the entry
gate never tested.**

Found by running Phase 2's own acceptance case before writing the fix for it.

**The probe.** `install()` creates an image, builds a `Picture`, `setGroup`s it, and
returns — dropping every binding it made. `main` then presents a `canvas::Group` naming it,
under the golden harness's environment (`MFB_*_HEADLESS=1`, `MFB_CANVAS_SYNC=1`,
`MFB_CANVAS_DUMP`). Result: `groups=1 groupBytes=461 blocks=1 draws=0:1:0:0:0 frames=1`,
and a frame that is **`(0,0,0,255)` × 576000 — every pixel black**.

**The control, which is the part that matters.** The same `Picture`, binding still alive,
handed straight to `canvas::present`. **Also entirely black.** So the first result says
nothing whatever about ownership.

**Root-caused, three independent ways, rather than inferred from the black frame:**

1. `helper_geometry.rs`: `CASE Picture(pic) RETURN __canvas_emptyHeader()`. A `Picture`
   produces the `NONE` geometry kind — *"a real kind rather than an absent record"* so the
   indices stay parallel — and every renderer skips `NONE`. This is not backend-specific:
   it is upstream of all three.
2. `canvas::imageHandle` has **no caller in any renderer**.
   `grep -rn imageHandle src/ | grep -v func_handle_bridge.rs` finds only the three
   per-target support tables, `data_objects.rs`, `module_analysis.rs` and two comments. Its
   twin `fontHandle` has six live callers in `helper_geometry.rs`. The font path exists;
   the image path does not.
3. `.ai/canvas-threading.md` §8 says so outright: *"Rows R1, R2, R9, R10 and R11 are not
   yet reachable. They are the texture and dirty-upload rows, and there is no texture:
   `Picture` draws nothing until plan-98-G brings the sampler, and `canvas::createImage`
   allocates nothing outside MFB's own resource record."*

**What this breaks in this letter.**

| Where | Text | Status |
|---|---|---|
| Phase 2 acceptance | *"the drop-the-binding case **draws the image**"* | **Unmeetable.** No `Picture` draws any image on any path. |
| Phase 3 acceptance | *"the 200-cycle loop shows **no fd growth** (`lsof`) …"* | **Vacuous.** `createImage` allocates nothing outside MFB's own record, so there is no fd to grow and a green result proves nothing. |
| Goal bullet 4 | *"200 install/remove cycles leak neither file descriptors nor **backing textures**"* | Same: there are no backing textures yet. |

**What it does not break.** The ownership work itself — the transitive move (**J10**) and
the free-path close (§4.3) — is entirely implementable and entirely testable now, because
its observables are the **resource record** and the **diagnostic**, not pixels:

* whether a binding is moved is `2-203-0055` at compile time, which **J7**'s and **J9**'s
  probes already read;
* whether a resource is closed is observable at run time through `canvas::getSize`, which
  raises `ErrResourceClosed` on a closed image — `closedRefuses` in
  `tests/cli_canvas_image_resource.rs` is the existing pattern;
* whether the group's buffer is freed is `groupBytes=` in `MFB_CANVAS_STATS`, which
  plan-116-G already made load-bearing.

**So the criteria are strengthened rather than dropped, per the rule that an acceptance
criterion that cannot be met as written is rewritten to something checkable and never
weakened.** "Draws the image" is replaced by an assertion on the image's **closed state**
after the producing scope exits — which is strictly more discriminating for what this
letter is about: a rendering assertion cannot tell *"owned correctly"* from *"nothing
draws"*, and the resource-state assertion can. The pixel assertion is **not deleted**: it
is recorded as a Prerequisites row against plan-98-E/G, so that when the sampler lands the
check is already written down rather than re-derived.

**And the Goal is corrected, not narrowed.** Bullet 4's *"neither file descriptors nor
backing textures"* is what it will mean once the sampler lands; today the leak that is
actually observable is the arena bytes the group owns, which `groupBytes=` reports. Both
are recorded.

**J10 (2026-09-04, Phase 2's first box, measured early while letter I's Linux row
compiled) — the transitive move IS reachable, the machinery is 80% there, and the 20%
that is missing is a distinction the checker does not currently draw.**

**What exists.** `TypeEnv::check_resource_moves` (`src/ir/verify/resources.rs`) already
carries an **alias graph** — `aliases: HashMap<String, HashSet<String>>` with an
`alias_closure` walk — added by plan-59-E for *"take a handle, give it back"*. A consume
marks the whole closure moved:

```rust
for alias in alias_closure(&consumed, aliases) {
    moved.insert(alias);
}
moved.insert(consumed);
```

So "consuming through one name consumes every name that may denote it" is solved, tested
(`rejects_double_move_close_then_return`, `move_in_if_branch_propagates_past_join`,
`foreach_body_move_leaks_to_outer`), and merges correctly across `If`/`Match`/`ForEach`
joins.

**Why it does not fire here.** The alias edge is recorded on `Bind` only when the bound
value is a call whose **return type is itself a resource with a close op**, and only for
arguments *of that same resource type*. `LET a AS canvas::DrawItem = canvas::Picture[…,
image := img, …]` returns a `canvas.DrawItem`; `close_op_for(DrawItem)` is `None`; no edge.

**And widening the alias relation to cover it would be a bug, not a fix.** Aliasing means
*may denote the same resource*, and a consume of either end marks both. Containment is not
symmetric: closing the image must **not** invalidate the item. Probed —

```basic
LET a AS canvas::DrawItem = canvas::Picture[…, image := img, …]
canvas::destroyImage(img)
canvas::present([a])
```

— **compiles**, and it has to: `Picture.image`'s own description promises *"closing it
while a scene still names it draws nothing rather than failing."* Recording `a` and `img`
as aliases would reject that program.

**So the missing piece is a second, directed relation.** Sketch, in the terms the code
already uses:

* a `contains: HashMap<String, HashSet<String>>` populated at `IrOp::Bind` when the bound
  type is a record (or a union of records, or a `List OF` one) with `RES` props and an
  argument is a local of that prop's resource type — the same shape as the existing alias
  edge, minus the symmetry;
* `consumed_resource` (`src/ir/verify/link.rs:975`) returns `Option<String>` today and
  would return a **set**: for a consuming-parameter call, the containment closure of the
  argument;
* a registry flag saying `setGroup`'s `items` parameter consumes what it contains.
  `RegistryResource` already carries `close_function`, `sendable`, `close_may_fail` and
  `live_slots`, so a per-`Parameter` consuming flag is the established shape rather than a
  new mechanism.

**Consequence for §Non-goals, which Phase 2 must correct rather than route around.** It
says *"No new `canvas::` surface. `setGroup`'s signature is unchanged."* The rendered
signature would indeed be unchanged, but its **meaning** would not: passing an item list
would consume the resources inside it. That is a semantic change to a public builtin and it
belongs in the man page, so the Non-goal as written is too strong and the letter should say
what it actually promises — *no new members*, not *no change to what `setGroup` does to its
argument*.

**Not a stop.** This is larger than §3 claimed (**J9**), and larger is explicitly not a
reason to narrow the Goal or defer the work.

**J9 (2026-09-04, Phase 1 — measured, three probes) — §4.3.1's recommended option is
right in outline and both of its cheap readings are dead; and §1's first Goal bullet
depends on a mechanism that does not exist, which the letter never says.**

All three probes are `mfb build -app` on a scratch project, against
`target/release/mfb`.

**Probe 1 — the hole is reachable today.** One image, two `Picture`s, two `setGroup`s:

```basic
RES img AS canvas::Image = canvas::createImage(1, 1, px)
LET a AS canvas::DrawItem = canvas::Picture[x := 0.0, …, image := img, paint := …]
canvas::setGroup("one", [a])
LET b AS canvas::DrawItem = canvas::Picture[x := 9.0, …, image := img, paint := …]
canvas::setGroup("two", [b])
```

**Compiles clean.** No diagnostic of any kind. §4.3.1 is not a hypothetical.

**Probe 2 — moving the list catches nothing.** The two installs pass *different* lists,
`[a]` and `[b]`. The binding used twice is `img`, in two separate constructions. A move
attached to `setGroup`'s `items` parameter moves a `List OF DrawItem` and leaves `img`
untouched. This is how I first phrased option 2, and it is wrong.

**Probe 3 — moving at the constructor is impossible, not merely undesirable.** The
alternative is to make `Picture[image := img]` consume `img`. But
`canvas::present(items AS List OF canvas::DrawItem)` and
`canvas::setGroup(name AS String, items AS List OF canvas::DrawItem)` take the **same
type** (`mfb man canvas present`, `mfb man canvas setGroup`), so a `DrawItem` cannot know
at construction which of the two it is destined for — and §Non-goals requires `present`
not to own. Probed the shape that would break:

```basic
FOR i = 1 TO 3
  LET p AS canvas::DrawItem = canvas::Picture[…, image := img, …]
  canvas::present([p])
NEXT
```

**Compiles today**, and it is the ordinary live-scene shape — one long-lived image, a
fresh `Picture` per frame. A consuming constructor refuses it on the second pass.

So option 2 survives only in a **transitive** form: the move is decided at the `setGroup`
call, and the checker walks argument → list → record → the `RES` slots inside. That is new
analysis in `src/ir/verify/`, and it is Phase 2's first box.

**The consequence for §1, which is the part worth stopping on.** Goal bullet 1 reads
*"`setGroup` takes ownership … so the caller's bindings may go out of scope without
closing them."* The second clause does not follow from the first. Scope-drop closes a
`RES` the binding still owns; the group holding an **alias** (**J5**) does nothing to
prevent that. For the caller's binding to go out of scope harmlessly it must have been
**moved** — so the transitive move is not a nicety option 2 offers for catching a sharing
bug, it is **the mechanism the Goal's headline promise is built on**, and without it
Phase 2's own test (open an image, `setGroup` a `Picture`, drop the binding, present — the
image still draws) fails no matter what the free path does.

The letter never says this. It describes ownership as a free-path concern throughout, and
§3 calls itself *"deliberately small"*. It is not small: it needs a move-checker change.
Recorded here rather than resolved because Phase 2 measures whether that analysis is
reachable, and **size is not a reason to narrow the Goal** — the Goal is what the letter
is for.

**J8 (2026-09-04, Phase 1) — §4.3 as I first wrote it attributed an argument to
plan-116-G that plan-116-G does not make; following the real one to the end found a hole
the letter's premise does not survive unpatched.**

**The mis-attribution.** §4.3 said plan-116-G justifies the ungated free at
`emit_retire_current_items` on the grounds that *the block was never published to the
graphics thread*. It does not. Its doc comment argues from **frames**: *"the drain gate
runs at the top of every `present`, so a still-occupied retired word means no frame has
completed since it was retired, which means no render can have started reading it."* I had
reconstructed a plausible argument instead of reading the one that is there — the failure
mode `.ai/…` line-citation decay produces, arrived at from the other direction.

Reading the real one made the concern evaporate rather than sharpen: **the close is
strictly weaker than the free already on that line.** Anything that could observe the close
must first read the block, and the free already asserts nothing can. §4.3 now says that,
and no longer hands Phase 2 a verification task that was an artifact of my own paraphrase.

**The hole, which is the part that matters.** That argument covers the group's own
reachability and nothing else, and *reachability is the wrong axis*. `canvas::Image` is a
handle to a **shared** record with one `closed` word. Two groups can hold `Picture`s built
from the same image; so can a group and the live scene. A group that "owns" its items and
closes them on free does not free *its* image — it closes *the* image, and every other
holder silently starts drawing nothing, because `imageHandle` answers `0` and `0` is "no
such object". The free path cannot detect it: it proves nobody is reading **this block**,
which is true and irrelevant, since the other holder reads a different block pointing at
the same record.

There is no refcount to fall back on and no latent one — §7 says *"there is nothing to
count"*, and `CANVAS_GROUP_REFS` is written and decremented but never read as a predicate
(**J4**). §Non-goals rules out adding one.

Recorded as §4.3.1 with three ways out, as a new Open Decision recommending the second
(`setGroup` moves the resource out of the caller's binding, as a direct `destroyImage`
does), and as two new Phase 2 boxes — the measurement that decides it, and a test pinning
whatever it decides. It is deliberately **not** settled in this document: option 2 turns on
whether a `List OF DrawItem` built inline reaches `setGroup` as something the move checker
can attribute to a binding, and that is measured, not reasoned.

**Why this is a Phase 1 finding and not a Phase 3 surprise.** Phase 3's box says *"the
group free path closes each owned resource once"*. Written as stated, against a shared
record, "once" is satisfied and the program is still wrong — the count is right and the
close is global. A phase whose acceptance criterion can be met by broken code is the shape
this correction exists to catch.

**Credit where it is due: §3 saw this and then dropped it.** Its risk paragraph says, in
as many words, that *"a group closing an image a scene still names would make that scene
draw nothing, which is a silent wrong picture"* — the same failure, one holder narrower.
So the finding here is not the hazard, which the letter already knew; it is that **the
hazard was named in prose and then addressed by none of §3's three pieces and no phase
box**, and that it is wider than stated (two groups, not just group-versus-scene). §3 also
claimed *"the rule that prevents it is already written"*, pointing at
`.ai/canvas-threading.md` §7 — and §7 does not prevent it. §7 is what makes the wrong
picture *silent*. Both are corrected in §3, and §4.3.1 is where the warning finally
reaches the design.

**J7 (2026-09-04, Phase 1) — verifying `.ai/canvas-threading.md` §7 against landed code
found one true claim stated too broadly, and disproved a constraint I had just written
into §4.2 myself.**

**The §7 claim, probed rather than reasoned about.** A scratch app-mode project that calls
`canvas::destroyImage(img)` and then builds a `canvas::Picture` from `img` is refused:

```
error[2-203-0055 TYPE_USE_AFTER_MOVE]: binding is used after move
              Binding `img` was moved and cannot be used again.
```

So the claim holds, and the mechanism is now recorded rather than assumed:
`destroyImage`'s parameter is `ParameterType::named(IMAGE_TYPE_ID)` — a plain
`canvas::Image`, **not** a `RES` one — so passing a resource to it is a move.

**But §7 and row R3 stated it without the qualifier, and the missing case is exactly this
letter's.** A `RES` parameter is an *alias* and consumes nothing, so a close performed
behind one leaves the caller's binding usable. That is not a hypothetical:
`closedRefuses` in `tests/cli_canvas_image_resource.rs` calls `closeIt(img)` — a
`SUB closeIt(RES img AS canvas::Image)` — and then `canvas::getSize(img)`, which
**compiles** and raises `ErrResourceClosed` at run time. A group closing its own items is
in that second category by construction, so a reader who took R3 at face value would
conclude this letter's close is protected by a compile error that cannot reach it.

Corrected in the doc, not just here: §7's bullet now names the parameter-type mechanism
and adds the "directly" qualifier with the test citation, and the matrix gains **R3b** for
the close-behind-a-`RES`-parameter case, protected by the runtime closed-read guard.
§4.6 records what this letter may and may not lean on.

**And the self-inflicted one.** §4.2, as first written this phase, claimed
`RESOURCE_OFFSET_CLOSED` is a flag set (bit 0 closed, bit 1 moved) and that the close must
therefore be a read-modify-write. **It is a plain boolean word.** `lower_destroy_image`
emits `move_immediate(flag, "1")` then `store_u64(flag, record, RESOURCE_OFFSET_CLOSED)`,
and the resource system's own `emit_closed_resource_record`
(`builder_value_semantics.rs`) does the identical whole-word store; no second bit is
defined anywhere in the tree. §4.2 is corrected. Left standing it would have put a
read-modify-write into Phase 3 to defend a hazard that does not exist — a design
constraint invented, at the point in the plan where inventing one is cheapest and
catching it is hardest.

**J5 (2026-09-04, pre-execution) — §2's open transfer question, answered, with the tail
audit behind it.**

The question: *does installing a resource into a process-global, graphics-thread-readable
group buffer constitute a transfer under plan-114's rules?*

**No.** A transfer is a move across a thread **plane** — `thread::transfer` /
`thread::accept` — which relocates the resource record between arenas and is why
`live_slots` exists: the copy must carry every live word past the canonical header.
Installing into the group table relocates nothing:

* the table stores a **pointer** to the items block;
* the block's `Picture`/`Text` slot holds a **pointer** to the resource record, which
  stays where the worker allocated it (`flatness_walk`'s `Res(_)` arm — the copy is an
  alias, **J4**);
* the graphics thread **reads** that record, which is the pattern already established for
  every published scene: those blocks are worker-arena memory the graphics thread reads
  (`.ai/canvas-threading.md` §3). Ownership adds no new cross-thread class.

So `live_slots` need not change and `sendable` need not be flipped. **This letter is not
a transfer; it is a lifetime extension within one arena.**

**The audit the box asks for, done anyway, because "if yes" is not the only reason to
know:**

| resource | tail past the header | live across a hypothetical transfer? |
|---|---|---|
| `Image` | `WIDTH` 32, `HEIGHT` 40, `PIXELS` 48, `DIRTY` 56, `LAST_USED_FRAME` 64 | **`PIXELS` is the source of truth** the backend re-uploads from, and `WIDTH`/`HEIGHT` describe it. A transfer declaring no slots would truncate all three. |
| `Font` | `BYTES` 32 | **the whole file.** Same conclusion, more starkly. |

Both are declared `live_slots: &[]`, and **that is not a landmine** — the source says so
in as many words: *"Not audited for transfer (bug-464 left canvas out of scope). Empty
here is only consistent with `sendable: false`; opting an image in means auditing its
record tail first, not just flipping the bit."* The `Font` twin adds *"which holds the
whole file"*. The declarations are honest placeholders, and the audit above is what a
future `sendable: true` would need — recorded here so it is not re-derived.

**One consequence for this letter's own design.** Because the group holds an *alias*
rather than a copy, "taking ownership" cannot mean "the group now has its own resource".
It can only mean **the group becomes responsible for closing the one that exists**. That
is why the close has to hang off the free path (**J4**: `emit_free_items_block`, the
chokepoint both free sites funnel through) and not off anything at install time — at
install there is nothing new to own.

**J4 (2026-09-04, pre-execution) — §1's description of the gate is wrong in two ways,
J2's central claim is wrong, and the slot this letter needs room in is full.**

Measured before starting, after plan-116-I landed.

**1. There is no `refs == 0` term in the gate, and there never was.** §1 says the free
*"already gates on `refs == 0 AND retiredFrame < lastCompletedFrame`"*. The actual gate,
`gen_group.rs` in `emit_group_reclaim`:

```rust
builder.emit(abi::load_u64(&retired, &slot, CANVAS_GROUP_RETIRED_ITEMS));
builder.emit(abi::compare_immediate(&retired, "0"));
builder.emit(abi::branch_eq(&next));          // discriminator: RETIRED_ITEMS != 0
...
builder.emit(abi::compare_registers(&frame_now, &stamped));
builder.emit(abi::branch_ls(&next));           // frame_now >= stamped
```

`CANVAS_GROUP_REFS` is **written `1` and decremented, and never read as a predicate
anywhere** — five references in the whole tree, none of them a test. That is deliberate:
`.ai/canvas-threading.md` says *"There is no refcount, and there is nothing to count …
the lifetime rule is the drain gate alone."* And the frame term is `frame_now >= stamped`
against the frames-**completed** counter, not `retiredFrame < lastCompletedFrame`. A
letter that hangs resource-closing off "when refs hits zero" would be building on a
counter nothing reads.

**2. There are TWO free sites, and the obvious one is not the dangerous one.**
`emit_group_reclaim` frees through the gate. But `emit_retire_current_items` **also**
frees — a prior retired block, deliberately bypassing the gate — so a second `setGroup`
in one frame goes down that path. A close step added only at the reclaim site leaves
those resources unclosed. Both funnel through `emit_free_items_block`, which is therefore
the chokepoint: a walk-and-close inserted before its `arena_free` covers both.

**3. J2's claim that `copy_flat_block` becomes illegal is wrong.** `List OF DrawItem` is
**still** `type_is_memcpy_copyable` after plan-116-I: `flatness_walk`'s `ParameterType::Res(_)`
arm returns true, because the slot holds one 8-byte pointer to the resource record and a
memcpy of that pointer is a correct **alias** (§15.6). The test J2 quotes,
`a_res_collection_does_not_diverge`, is about `List OF RES fs.File`, where the element
type *strips* the `RES` marker — the opposite case. So the copy at `emit_set_group` stays
legal, and that is the problem rather than the solution: the group gets an alias, and
`arena_free` on the list block then drops the last reference to records nothing closes.

**4. The group slot is full.** `error_constants.rs` says so in as many words: *"Both spare
words are now used: `RETIRED_ITEMS` and `RETIRED_NAME`. plan-116-J will need to grow the
slot to 128 (still a power of two, still a shift) rather than find room here."* Any new
word must also be added to `the_group_slot_size_is_a_power_of_two_matching_its_shift`,
which takes the max over a hardcoded list of the eight words.

**5. Nothing walks a group's STORED items.** Every existing walk goes through
`canvas::groupItems(slot)`, which returns a **copy**, and runs on the graphics thread —
`__canvas_appendDraw`, `__canvas_groupSignature`, `__canvas_memoGroup`,
`__canvas_drawGroup`. None visits `Picture.image` or `Text.font`; the only code that reads
a resource out of an item is `helper_geometry.rs`'s six `canvas::fontHandle` sites. This
letter has to write that walk, and it has to run on the **worker**.

**6. The existing owned-container machinery does not cover this case.** An owned list
carries **one** `OwnedListDrop`, and `builder_resource_cleanup.rs` explicitly refuses *"a
record with two `RES` fields of differing resource types"*. A `DrawItem` list holds both
an `Image` (via `Picture`) and a `Font` (via `Text`) — two close ops — so
`emit_owned_list_drain` cannot be pointed at it unchanged.

**Cheap, at least:** closing one resource is two instructions —
`move_immediate(flag, 1)` then `store_u64(flag, record, RESOURCE_OFFSET_CLOSED)`. There is
no runtime call; the OS-side free is already deferred behind the backend's own
`closed AND lastUsedFrame < lastCompletedFrame` gate. `destroyFont` adds one step
(`emit_unregister_font` before the flag, so a renderer never finds a published block whose
resource is closed). The cost of this letter is the walk, not the close.

**Stale citations, corrected.** `Picture.image` is now `RES canvas::Image` and `Text.font`
`RES canvas::Font`; the two `pkg.add_resource` calls moved to `mod.rs:981` and `:1002`
(J2 and J3 both give older numbers — G1's lesson repeating twice in one document). Two
strings §Non-goals quotes as documented promises **no longer exist** in the tree: *"an
installed scene never keeps an image open"* and *"keeps the scene from retaining
anything"*. The nearest survivor is `src/docs/spec/app/06_canvas.md`'s *"Naming a
resource in a scene does not keep it alive"*, which plan-116-I wrote. The Non-goal itself
still stands — `present` must not own — but it needs to cite something that exists.

**Also stale, in the source rather than the plan:** `gen_present.rs`'s comment still
claims a collection is *"a self-contained flat block … so `copy_flat_block` is already
the transitive deep copy"*, and `gen_image.rs` still calls `handle@8` *"the only thing a
scene ever carries (through an `ImageRef`)"*. Both are now wrong in the same way, and
both are load-bearing prose for exactly the question this letter asks.

**J3 (pre-execution, 2026-09-04) — re-measured §2; the counts hold, but one row's
command over-counts by 2× and would have been read as drift.**
`grep -c add_resource src/codegen/builtins/canvas/mod.rs` returns **4**, against a
recorded count of 2. There has been no drift: two of the four hits are prose — the
module comment at `:25` explaining that `add_resource` derives a runtime call from its
close op, and an inline comment at `:993` making the same point. The real declarations
are `pkg.add_resource(RegistryResource {` at `:1002` and `:1023`, which is 2.

The row now anchors on `pkg.add_resource`. Worth a correction rather than a quiet edit
because of which way this error runs: a census command that over-counts invites the next
reader to *widen* the plan's scope to cover a population that does not exist, and to go
looking for two resources that were never declared. Project memory records the opposite
failure — a census by one helper name undercounting ~6× — and this is its mirror. Both
come from grepping a token that appears in prose as well as in code.

The other five rows re-measure correct: `TYPE_RESOURCE_FIELD_FORBIDDEN` still reserved-
not-emitted at `src/rules/table.rs:1010`; `ls planning/completed/plan-114-*` → 5;
`Picture.image` still `ImageRef` and `Text.font` still `FontRef` (plan-116-I has not
run); `live_slots: &[]` and `sendable: false` on both resources.

**J2 (2026-09-03, pre-execution, measured against plan-116-G as landed) — G's `setGroup`
copies its item list with `copy_flat_block`, which is the wrong primitive the moment
plan-116-I puts a `RES` in a `DrawItem`.** This is the concrete shape of §2's open
question, and it is a defect that does not exist yet — it appears when I lands, in code
G already wrote.

`emit_set_group` (`src/codegen/builtins/canvas/gen_group.rs`) deep-copies the incoming
`List OF DrawItem` with `builder.copy_flat_block(&items_type, …)`, which is correct today
because no `DrawItem` carries a resource. `builder_collection_layout.rs` pins the rule
that stops being true:

> *"what keeps a resource-carrying collection out of `copy_flat_block` and out of
> `is_freeable_flat_value` — both of which would be wrong for it"*
> (`a_res_collection_does_not_diverge`)

So J's Phase 2 — *"`setGroup`'s deep copy routes resource ownership per Phase 1's design
instead of copying a handle"* — is not an addition to that call, it is a **replacement of
it**. And J's Phase 3 inherits the same on the way out: `emit_free_items_block` frees the
retired buffer as a flat block, which is `is_freeable_flat_value`'s other half.

**§2's open question is answered by the two resource declarations themselves**, which say
more than §2 does. Both `canvas::Image` and `canvas::Font` declare `live_slots: &[]`, and
both comments state the reason it is sound and the condition that ends it — the `Font`
one being the sharper:

> *"opting a font in means auditing its record tail — **which holds the whole file** —
> rather than flipping the bit."*

So the tails are **not** empty; `live_slots: &[]` is an assertion that is only consistent
with `sendable: false`. Installing a resource into a process-global buffer that the
graphics thread reads is exactly the case `sendable` governs, so the answer to §2's *"does
this constitute a transfer"* is **yes**, and the audit §2 asks for is a prerequisite of
Phase 2 rather than a follow-up to it.

*(Citations corrected: the declarations are at `mod.rs:1016` (`Image`) and `:1037`
(`Font`), not `:748`/`:790`. `grep -n 'live_slots' src/codegen/builtins/canvas/mod.rs`
finds both.)*

**J6 (2026-09-03, pre-execution; recorded as a second "J1" and renumbered 2026-09-04 —
two corrections carried the same number, so a reference to "J1" resolved to whichever
the reader found first) — the rule-retirement row cited a line range that no
longer contains what it describes.** The check was `sed -n 1008,1019p src/rules/table.rs`
"under a *retired by plan-114-B* comment". The `Rule` block for `2-203-0084` does start
at 1008, but the comment explaining the retirement sits **above** it (`:1004-1007`, "Kept
rather than deleted so the code is never recycled for a different meaning"), so the cited
window shows the rule and not the reason — a reader running it sees an ordinary,
live-looking rule row.

Replaced with a check that cannot drift and that tests the *claim* rather than a
location: `grep -rn TYPE_RESOURCE_FIELD_FORBIDDEN src | grep -v rules/table.rs` returns
no emit site — only two doc comments in `ir/verify/`, the spec's history note, the
rule-codes table marking it *"reserved, no longer emitted"*, and
`ir/verify/tests.rs:3258`, which asserts the code is **not** produced. That last one is
the real guarantee: the retirement is pinned by a test, not merely by a comment.

Confirms the row, so the status is unchanged — but a Prerequisites command that no longer
shows what it claims is one a reader may reasonably mark NOT MET, and this letter cannot
start if a row reads that way.

**J1 (2026-09-03, pre-execution) — every `mod.rs` line citation in this letter is
stale, the same defect plan-116-G recorded as G1.** Checked with
`awk 'NR==N {print}' src/codegen/builtins/canvas/mod.rs` for each cited N; not one
lands on what the letter says is there. A sample:

* `mod.rs:748`, given twice as `Image`'s record tail and `live_slots`, is a bare
  `RecordProp {` opening brace.
* `mod.rs:385`, given as *"This is what keeps the scene from retaining anything"*,
  is a `CapStyle.Round` description; the real comment is at `:447`.
* `mod.rs:744`, given as the `Image` resource, is prose about arc sweep direction.
* `src/rules/table.rs:748`, given as `TYPE_THREAD_RESOURCE_PLANE_REQUIRED`, is a
  comment about a *different*, retired rule (`2-203-0102`). The rule this letter
  depends on is real and landed — `2-203-0138`, at `:741-746` — so the claim held
  and only the pointer was wrong, which is the pattern throughout.

The letters were written before plan-116-C, D, E and F each added records and
descriptions to that file. The counts and claims these citations *support* are not in
question — this is a navigation defect — but it is the dangerous kind, because the line
a reader lands on is plausible code they could edit in good faith.

Every one is replaced with the **symbol** and the command that finds it, per G1's
lesson: every letter of this plan edits `mod.rs`, so a line citation into it decays the
moment the letter before it lands. Verify with
`grep -n 'name: \"Picture\"\|live_slots\|keeps the scene from retaining' src/codegen/builtins/canvas/mod.rs`.

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
record-tail audit `mod.rs`'s `Image` resource (`live_slots`) says they have never had — and whether
`.ai/canvas-threading.md` §7's "presenting a stale handle draws nothing" survives
`Picture` becoming a resource. Both are scheduled as Phase 1 reading tasks, because
both are assumptions that would otherwise be inherited silently.
