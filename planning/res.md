# res.md — resource ownership & STATE: working notes

Last updated: 2026-07-16
Status: **THINKING — not a plan.** No design is committed here. This captures what
was established, what was disproved, and what is still open, so the thinking does
not have to be redone.

> **Track A is DONE.** `plan-52-A..D` are implemented and archived to
> `planning/old-plans/`. §4's table is now spec §15.5; §10's reclamation is real
> (961 MB → 31 MB on a 20k-cycle loop); the §2 fact table below still holds except
> where noted inline. **Track B (§1/§3, resource-scoped ownership) remains open and
> unaffected** — plan-52 forecloses nothing.
>
> Three things this doc got wrong, corrected by implementation:
> - **Fact #10's "nothing frees the record"** is still true of the *record* (by
>   design — it is the tombstone) but no longer of what it points at.
> - **§5 Q1** ("does the STATE payload follow the record's rule?") — resolved: the
>   state is freed at **drop**, the record is not. No double-free question arises;
>   the blocks are reachable only through the record, so nulling-as-we-free gives
>   once-only.
> - The **disclosure primitive** §6 worried about (via plan-52-C §2) **does not
>   exist**: a record's String field is a block-relative offset, so reading it as an
>   Integer leaks the constant `8`, not an address.
>
> Two new defects came out of doing the work: `bugs/bug-257` (thread::transfer
> admits a cross-thread STATE disagreement — **open**, and §5 Q5 is its ancestor)
> and `bugs/bug-256`/`bug-258` (both fixed).

Grew out of a review of `bindings/libsnd`'s `openFile` (wanting `sf_open` to hand
back an `SNDFILE*` carrying its `SF_INFO`). That question opened a much larger one:
**is a resource owned by a binding, or by a scope?**

The three defects this uncovered are now `planning/plan-52-A..D`, not bugs. See §7.

---

## 0. In small words

**How it works now.**

Every `RES` binding is an owner. An owner has one job: close the thing when I end.

You can hand a resource to a function, but only as a loan. The function can use it and
change its state. It cannot close it, cannot keep it, cannot hand it back.

Only the binding that made the resource can give it away, with `RETURN`.

Because each binding owns one known thing, the compiler can tell you **at build time**
that you used a file after you closed it.

**How it would work after the change.**

The resource has a scope: the outermost one that ever touches it.

Bindings are just pointers. They own nothing and close nothing. Hand one to a function,
return it, copy it into a list — all the same. All pointers to one thing.

When that outermost scope ends: close the thing, if it is still open. Once. The memory
goes back when the thread's arena is torn down.

The compiler can no longer always tell that two pointers are the same thing. So "used
after close" stops being a build error and becomes a **runtime** error.

**Why the second one felt like the truth.**

The language's author read this code and expected the second model. That is worth
writing down rather than filing as a mix-up: §15.6 already says ownership floats to the
outermost scope that references a resource, and never moves down. That *is* the second
model. It is simply fenced to collections. If the rule everyone reads is the second
model, the fence is the surprise — not the rule.

---

## 1. The question

Two coherent models. The language currently implements the first, and §15.6 already
implements the second for collections only.

**Binding-scoped (current).** Ownership is an obligation held by a binding. A param
is a borrow and can never escape. `AS RES File` on a return means "ownership moves
to the caller." Invalidation is a static property of a binding, visible at the call
site.

**Resource-scoped (proposed).** The *resource* has a scope: the outermost scope that
touches it. Bindings are pointers — they are never closed, never dropped, they just
point. When the resource falls out of scope: close if open, then reclaim. Aliasing is
a non-event because a pointer is a pointer.

The proposed model is not a misunderstanding of the current one. It is a different
design point, and §15.6's existing float rule is already a partial implementation of
it:

> Ownership always floats to the outermost scope that references the resource; **it
> never moves down.** If a referencing collection escapes the function (it is
> RETURNed), ownership moves out to the caller, exactly like RETURNing the resource
> itself.

That *is* the resource-scoped model. It is just fenced to collections.

---

## 2. Verified facts

Every row tested against `target/debug/mfb`, macOS aarch64, clean `build/`, using
built-in `File` (no LINK involved). Scratch programs, not kept.

| # | Behavior | Result |
|---|---|---|
| 1 | Bare `RES p AS File` param accepts a stateful argument; state survives the call | works (`pos still = 42`) |
| 2 | `.state` **write** through a bare param | rejected — `TYPE_STATE_INVALID`: "`p` has no STATE to assign" |
| 3 | `.state` **read** through a bare param | rejected, but degrades to `Unknown`; error lands on the consumer (`toString`), never mentions STATE |
| 4 | Stateful param on a **stateless** owner | **attaches** — allocates the state. Compiles, runs |
| 5 | Two different STATE types on one resource | **type confusion**, no diagnostic (→ plan-52-C) |
| 6 | `AS RES File STATE Cursor` + `RETURN <stateful>` | `TYPE_RETURN_MISMATCH` (→ plan-52-D) |
| 7 | STATE on a return, returning a **stateless** value | compiles — the annotation is fully **inert** |
| 8 | Union-STATE / non-defaultable-STATE on a **return** | both compile — verify rules never fire (→ plan-52-D) |
| 9 | `RETURN` of a borrowed param | rejected — `TYPE_RESOURCE_BORROW_INVALIDATE` |
| 10 | **A resource record is never freed** | no `arena_free` of a File/Socket record anywhere |
| 11 | `RESOURCE` declaration | has **no** STATE clause (`src/ast/items.rs:540-567`) |

### The load-bearing one — #10

`lower_fs_close_helper` (`src/target/shared/code/fs_helpers_io.rs:840`) flushes,
closes the fd, sets `CLOSED = 1`, returns. **It never frees the record.** At a `RES`
bind (`builder_control.rs:255-297`), the cleanup chain registers
`ActiveCleanup::Resource` (call the close op); the `OwnedValue` → `arena_free` branch
is an `else if` a resource never reaches. Nothing else frees it.

So the fd is released and **the record lives on holding its closed flag** (offset 8,
plan-38). Reclaimed only at arena teardown — which is **per-thread** (confirmed by
the user), not per-process, so retention is bounded by thread lifetime.

---

## 3. What the debate settled

### 3.1 The proposed model's cost is *not* double-free

The argument against resource-scoped ownership ran: if `a` and `b` both point at `R`
and both die in one scope, who frees `R` once? Close is idempotent (the flag); **free
is not**, and the `arena_free` path is sound today only because of an explicit
invariant:

> Copy-insertion (`lower_value_owned`) guarantees this block is **unaliased**, so the
> free is sound and once-only.

**That argument is void.** Per fact #10, resources are never freed, so there is no
double-free to protect against. The `unaliased` invariant guards `arena_free` for
*flat values* (String/List/Map) — a different path the model does not touch.

Worth recording how this was reached: the user independently proposed leaving "a small
sentinel in place, so everything points at it and knows it's been freed… it lasts the
life of the program, so it's a memory leak." **That sentinel already exists — it is
the resource record.** The closed flag is the tombstone. The design was already the
one being derived, and the cost already paid.

### 3.2 The actual cost is static use-after-close

This is a compile error today:

```basic
RES f AS File = fs::open("app.db", "read")
fs::close(f)
exec(f, "...")        ' COMPILE ERROR: f used after close
```

Under the resource-scoped model it cannot be. If `b = test(a)` may alias, the compiler
cannot know which bindings a close killed, so it cannot flag the use. It degrades to a
runtime `ErrResourceClosed` — safe, defined, but runtime.

That is the whole bill. §15's *"invalidation… all visible at the call site"* is the
thing being bought; the borrow rule is the price. It is a design choice, not a law.

### 3.3 Why the float rule stops at collections

Not because collections are special. Because §15.6's procedure is *"a purely syntactic
per-function decision procedure"* whose inputs include **"the insertion-builtin set"**
— a fixed set of builtins whose aliasing is hard-coded. `collections::append(list, f)`
is in it; the compiler knows exactly what it does with `f`.

An arbitrary `test` is not and cannot be. Given

```basic
RES b AS File = test(a)
```

the compiler cannot tell whether `b` aliases `a` or is a fresh resource — the
signature `AS RES File` does not encode identity. Both bodies are legal under one
signature.

Extending the float therefore needs one of:

- **runtime pointer identity** at scope exit (compare each binding's pointer against
  the escaping one; skip matches). Cheap — n is tiny. **Now the leading candidate**,
  since #10 removes the free problem;
- whole-program aliasing analysis (kills separate compilation);
- a signature that says *"the returned resource **is** parameter f"* — a lifetime /
  identity annotation. §15 rules this out: *"There is no user-visible lifetime
  construct."*

### 3.4 Two kinds of STATE

The existing STATE feature is **user-attached**: `File` has no intrinsic state; the
*user* hangs a `Cursor` on it, and different code may hang different things. §15's
wording is deliberate — "a RES binding **may** attach."

`bindings/libsnd` wants **constructor-attached**: every `SNDFILE*` has an `SF_INFO`,
defined by the library, not the caller. There is no stateless SfFile.

One mechanism currently serves both, which is the tension behind §4. The user
has **decided against** putting STATE on the `RESOURCE` declaration (recorded in
plan-52-A's Non-goals) — it would give one declaration site and make disagreement
unrepresentable, but forfeits the bare-param opt-out that close ops rely on.

---

## 4. The visibility model (now plan-52-A §3)

Proposed by the user; `RESOURCE SfFile CLOSE BY …` carries no STATE.

| Position | `RES x AS SfFile` | `RES x AS SfFile STATE FileInfo` |
|---|---|---|
| **Param** | any state or none; `.state` **not** accessible | **only** a SfFile carrying FileInfo; `.state` accessible |
| **Return** | a resource with **no** state | a resource **carrying** a FileInfo |
| **Binding** | **no** state | attaches/adopts a FileInfo |

Bare means **"opaque"** as a param and **"none"** everywhere else. Sound only because
a borrow cannot escape (fact #9) — the opaque reading is confined to the frame that
borrowed it.

**The rule:** bare erases state only where the resource *cannot escape* (params).
Where it can escape (bindings, returns), bare must mean provably no state.

Note the interaction: **§3 (resource-scoped) and this table are in tension.** The
table's soundness rests on the borrow rule; the resource-scoped model weakens it.
If both are wanted, this needs re-derivation. **Unresolved.**

---

## 5. Open questions

1. ~~**Does the STATE payload follow the record's rule?**~~ **RESOLVED (plan-52-B).** No:
   the state is freed at **drop**; the record is not. The double-free question does not
   return, because the payload is reachable **only through the record**, so nulling the
   pointer as it is freed gives once-only with no aliasing analysis — unlike
   `OwnedValue`'s `arena_free`, whose soundness needs copy-insertion's unaliased
   guarantee. `resource-state-drop-valid`'s comment was aspirational when written; it is
   true now.
2. ~~**Is per-thread retention actually bounded in practice?**~~ **MEASURED
   (plan-52-B).** It was worse than suspected and is now bounded per resource: 961 MB →
   31 MB on a 20 000-cycle loop, i.e. ~48 KiB/cycle → ~1.1 KiB/cycle, of which the two
   80-byte tombstones are 160 B and the rest is arena bookkeeping. Retention no longer
   scales with the I/O a resource did. The remaining 80-bytes-per-resource-ever-opened
   is the deliberate tombstone cost (§10); revisit only with a real workload.
3. **Does the resource-scoped model survive the §4 table?** Per §4's note. The most
   likely resolution: keep the borrow rule (so §4 stands) and *don't* pursue §3 —
   i.e. the two are alternatives, not a package.
4. **Is losing static use-after-close acceptable?** The honest trade. Aliased handles
   are how the underlying C libraries work anyway.
5. **`thread::transfer`** — **audited (plan-52-A Phase 3); now `bugs/bug-257`, OPEN.** The
   suspicion was right: the plane copies the state pointer without consulting either
   side's type string, and a sender/worker STATE disagreement type-confuses across the
   boundary (verified at runtime). **plan-52 does not close it** — `thread::accept` is
   statically a bare `File`, so no static rule over type strings can see the STATE
   arriving. Needs the STATE on the plane type: its own plan.

   The "who retains it" half is also answered, and is the sharper part: the transfer
   allocates a **fresh record in the receiver's arena** but copies the STATE **pointer**
   verbatim, so the payload still lives in the **sender's** arena. The receiver holds a
   cross-arena pointer whose lifetime is the sender thread's. plan-52-B's `moved` bit
   stops the sender's drop freeing it; nothing stops the sender's *arena teardown*.
   Recorded in bug-257.

---

## 6. Reasoning errors made while deriving this

Kept deliberately. Each was stated confidently and was wrong; each is a trap the next
reader can fall into the same way.

- **"Not decidable"** (said of the resource-scoped model) — **overstated.** Not
  decidable *statically*. Runtime pointer identity makes it implementable. §3.3.
- **"The core problem is who frees R once"** — **wrong.** Nothing frees R. §3.1. This was
  argued as the model's central flaw; it does not exist.
- **"`stateful → bare` is allowed — recommend yes"** — **wrong for bindings**, right for
  params. Reasoned from the param case (safe, because a borrow cannot escape) and
  generalized without noticing that a binding *can* escape. That laxity is a laundering
  primitive. §4, plan-52-D §3.
- **"`stateless → stateful` is fine (allocate)"** — **contradicted by fact #4.** Param-attach
  is what makes two disagreeing borrows reachable with no stateful binding anywhere. The
  over-rejection risk claimed for it is phantom: no in-tree fixture depends on it
  (`resource-state-field-assign-valid`'s *owner* declares the STATE). plan-52-C §3.
- **"Keep the header, free the contents"** — **near-worthless as stated.** Proposed while
  believing the record was ~24 bytes; it is 80, and the header *is* the record. §10.
- **The stale-binary false finding** — a failed `mfb build` leaves the previous
  `build/*.out` in place and running it looks like a pass. This produced one finding that
  was recorded and then retracted. `rm -rf build`, check the timestamp. plan-52-C §2.

The common thread in the middle two: **the escape distinction** — params may erase state
because they cannot escape; owners may not. Both errors come from reasoning about one
position and generalizing to all four.

---

## 7. The three defects — now plan-52, not bugs

**All three are fixed** (`plan-52-A..D`, archived to `planning/old-plans/`). The section
below is kept as written, for the reasoning about *why* they were tracked as plan work
rather than as bugs.

These were filed as `bug-252/253/254` and then **removed**. Nothing was fixed and nothing
was lost: their entire work-plan became `planning/plan-52-A..D`, so keeping them as open
bugs meant two documents tracking one body of work — which promptly drifted (252 and 253
went stale inside a single session; see §6). Not fixed + fully planned = **plan work**, not
a bug. If any of these is later found in the wild as a *symptom* rather than as planned
work, file it fresh against the plan.

- **A `FUNC` cannot return a resource carrying STATE** → **plan-52-D**.
  `src/ir/lower.rs:724-730` and `:1963-1970` never append `return_state_type` to the return
  type string, unlike params (`:739-742`) and bindings (`:974-977`). The same omission lets
  union-STATE and non-defaultable-STATE escape verification on a return.
- **STATE type confusion** → **plan-52-C**. Allocation is guarded by a **null-check, not a
  type-check**; the payload is untagged at runtime. Reachable from safe source, no LINK, no
  threads. The most serious of the three.
- **The STATE visibility model** → **plan-52-A** (§4 above). The governing contract; the
  other two are its implementation gaps.
- **The reclamation leak** (§10) → **plan-52-B**. Never filed as a bug; found here.

---

## 8. Pros and cons of the change

**Pros**

- **It matches what a resource is.** A pointer to a thing. Copying a pointer is free and
  should mean nothing. Today it means something, and that is the surprise.
- **One rule instead of two.** The collection carve-out (§3.3) disappears. Float applies
  everywhere, for the same reason everywhere.
- **"Take a handle, give it back" becomes writable.** Not expressible today in any form.
- **Fewer ideas to hold.** No owner-vs-borrow split, no `TYPE_RESOURCE_BORROW_INVALIDATE`,
  no `TYPE_RESOURCE_ELEMENT_NOT_OWNER`.
- **It matches the C libraries underneath**, which alias handles freely and always have.
- **It is what the author expected** (§0). That is evidence about which model is the
  intuitive one, from the person best placed to have the intuition.
- **It is cheap.** The hard part — free exactly once — does not exist, because the record
  is never freed (fact #10, §3.1).

**Cons**

- **Loses static use-after-close** (§3.2). `fs::close(f)` then `exec(f, …)` becomes a
  runtime `ErrResourceClosed` instead of a build error. This is the real bill.
- **Invalidation stops being visible at the call site.** `fs::close(b)` kills `a`, and
  nothing where `a` lives says so. §15 explicitly buys the opposite — that property is
  the whole reason the borrow rule exists.
- **Needs runtime pointer compares at scope exit** (§3.3). Cheap — n is tiny — but it is
  new machinery on every scope exit holding resources, where today there is none.
- **§4's STATE model would need re-deriving.** Its soundness rests on borrows not
  escaping. Weaken that and the table has to be re-argued from scratch.
- **Real churn**: `escape.rs`, two rejection rules, §15 of the spec, and the mental model
  in every doc that touches resources.
- **It blocks the STATE work.** plan-52-A..D assume the current borrow rule. Doing this
  first stalls `bindings/libsnd` behind a language redesign; doing it second costs nothing.

**The shape of it:** the pros are mostly about *simplicity and honesty* — one rule, matching
the machine, matching the intuition. The cons are mostly about *what the compiler can prove
for you*. That is the trade, and neither side is obviously right.

---

## 9. Where this is headed

Two independent tracks, and they should probably stay independent:

**Track A — finish STATE (`plan-52-A..D`). DONE.** The §4 table, made real: it is spec
§15.5, the rules are enforced, and `bindings/libsnd`'s wrapper shape compiles. Depends
on the borrow rule staying as it is — which it does.

"Mostly mechanical" was optimistic. The model and the rules were; what the work actually
cost was everything the type string touched once a *return* could carry a STATE — the
`.mfp` ABI export encoding, syntaxcheck's `Type`, the native storage class, and the
poison in a non-`File` record's buffer words. See the archived sub-plans' Status headers.

**Track B — resource-scoped ownership (§1, §3).** A language-design change. Cheaper
than it looked (§3.1), costs static use-after-close (§3.2), needs runtime pointer
identity (§3.3). Would require re-deriving §4.

**A does not need B.** Doing A first is the low-risk path, and B remains open
afterward. The reason to consider B at all is that the current model cannot express
"take a handle, give it back," and the float rule's collection-only fence is arbitrary
from the outside.

**Track A is now DONE: `planning/old-plans/plan-52-A..D`.** It carried the STATE model
(§4), bugs 252/253/254, and the reclamation work (§10 below) — and explicitly
excluded Track B. plan-52-B assumes the current borrow rule but does not depend on it,
so it stands either way.

Track B is still mulling. **Nothing in plan-52 forecloses it** — but note what doing A
first bought: §3.1's central argument (that resources are never freed, so the
double-free objection is void) is now *half* false. The record is still never freed, so
the argument survives where it matters — but the STATE payload and the buffers now ARE
freed at drop, and their once-only property rests on being reachable **only through the
record**. If Track B makes bindings plain pointers to a shared record, that property is
unchanged (the blocks still hang off one record) — so B's cost is still §3.2's static
use-after-close, not a free problem. Re-derive §4 before starting B either way.

`bugs/bug-257` is a live argument *for* thinking about B's neighbourhood: the resource
plane already aliases a record across arenas, unchecked.

---

## 10. Record retention — decided

Settled while working §5 Q2. **Implemented by plan-52-B** — the decision below is what
shipped, and it held up: measured 961 MB → 31 MB on a 20 000-cycle open/close loop. The
"What leaks today" past tense below is now historical; what remains retained is the
80-byte record, deliberately.

**What leaks today.** Nothing frees the 80-byte record
(`RESOURCE_RECORD_SIZE_BYTES = 80`, `error_constants.rs:689` — one uniform size for
*every* resource kind, per-backend asserts in `audio/`, `tls/`). Nothing frees its
`BUF_PTR` output buffer, its `READ_PTR` read buffer, or its `STATE` payload either —
grepping those against `free` returns nothing. So a resource retains 80 bytes **plus
its buffers** for the life of the thread. Worse than §5 Q2 suspected.

**A resource can never be large.** The record is fixed at 80 bytes and a user cannot
add fields (`ResourceDecl` is `{visibility, name, close_fn, thread_sendable, line}`).
A "50 MB resource" is really an 80-byte record pointing at big blocks — buffers, a
STATE payload, or the native library's own memory (freed by its own close op, never in
the arena). Every design frees those identically; retention only ever concerns the 80.

**Rejected: a handle LUT.** A growable heap table of `Integer` entries; `RES` holds an
index; flags in the spare bits (`Integer` is **64-bit**, so ~62 bits are free — plenty
for a pointer plus flags plus a generation counter). Would let the record itself be
freed, retaining 8 bytes instead of 80, and with generation-based slot reuse would
bound retention by *peak concurrency* rather than total-ever.

Rejected because: (i) a dependent load + mask on **every** resource access, including
`.state`, where today there is a direct pointer; (ii) arenas are per-thread, so
`thread::transfer` must re-register the record in the receiver's LUT and rewrite the
index, while the sender's entry still points at the record — a cross-thread double-free,
the exact hazard the flags were for. A `moved` flag on the sender's entry fixes that,
but only after the machinery is already built; (iii) a global LUT needs atomics on grow,
cutting against "threads do not share… resources". 72 bytes per resource is not worth
it.

**Decided: keep the record, free what it points at, at drop.**

- The 80-byte record **is** the tombstone. It holds the closed flag that makes re-close
  idempotent and that every alias reads. This is the sentinel §5's thinking reached for
  — and the language already had it.
- Free `BUF_PTR`, `READ_PTR`, `STATE` at **drop**, nulling each as it goes. Those blocks
  are reachable only through the record, so nulling gives once-only for free — no
  aliasing analysis, unlike `OwnedValue`'s `arena_free`, whose soundness rests on
  copy-insertion's unaliased guarantee.
- **Not at close.** The `.state` read path (`builder_value_semantics.rs:175-190`) has no
  closed-guard: it loads offset 16 unconditionally. Freeing the payload at close would
  make `x.state` after `fs::close(x)` a null dereference. Close releases the OS handle;
  drop reclaims memory. §15 already treats them as separate events — the split holds.
- **`moved` = bit 1 of the CLOSED word.** The word is a u64 storing 0/1 with 63 bits
  spare, so no size growth and plan-38's offset-8 invariant survives. Every existing
  guard is `load; compare 0; branch_ne`, so it refuses a moved resource **for free** —
  only the paths wanting a distinct `ErrResourceMoved` change.

**Retention after:** 80 bytes × resources-ever-opened, per thread, reclaimed at arena
teardown. A thread opening 10M resources holds 800 MB. If that is ever a real workload,
the LUT + generation counter is the answer — bounded by peak concurrency. Revisit only
with a real workload; recorded in plan-52-B's Open Decisions.
