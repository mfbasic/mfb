# plan-30-D: Swift async ABI bridge (hand-rolled)

Last updated: 2026-07-07
Effort: medium (1h–2h)   <!-- highest correctness risk in plan-30 -->

This sub-plan hand-emits the **swiftcc** calling convention — including the **async
ABI** — for a *fixed, small* set of Swift entry points, so mfb can call StoreKit 2's
Swift-only async API directly with **no shim and no external compiler in the shipped
build**. It delivers a reusable native mechanism: "invoke async Swift function `F`
with marshalled args, block the calling (worker) thread until the result `R` is
available, return `R` marshalled into mfb values." Every emitted instruction sequence
is reverse-engineered from, and byte-validated against, the `swiftc` **development-time
oracle** (write reference Swift → `swiftc -emit-assembly` → diff mfb output until
equivalent). The shipped `mfb` invokes `swiftc` never.

The single behavioral outcome: mfb-emitted native code calls a real `async throws`
Swift function on the simulator and returns its result correctly, with ARC balanced
(no leak, no over-release) and the calling thread correctly suspended/resumed across
the `await`.

It complements:

- `mfb spec threading` (`src/docs/spec/threading/**` — the worker/main model the blocking bridge lives within)
- the `swiftc` oracle probes in this plan's §2 (recorded, not shipped)

## 1. Goal

- A swiftcc **sync** call path: correct `self`/error-slot register discipline, ARC
  insertion, value-witness/metadata-accessor referencing — validated against the
  oracle for a concrete (non-generic) sync Swift call.
- A swiftcc **async** driver: a single top-level `Task`, async-frame
  alloc/dealloc (`swift_task_alloc`/`swift_task_dealloc`), a continuation that
  unblocks the worker, and executor hops (`swift_task_switch`) — driving a real
  `…Tu` async function pointer to completion.
- Marshalling for the shapes StoreKit needs: mfb `String[]` → Swift `[String]`,
  Swift `Array<T>`/enum results → mfb values, `throws` → mfb error/`TRAP`.

### Non-goals (explicit constraints)

- **Not** a general Swift compiler. Only the *fixed* call shapes 30-E needs; each is
  hand-authored and oracle-diffed, not generated from arbitrary Swift.
- No `swiftc`/`clang` in the shipped `mfb` build or in any user's build — oracle only.
- No change to mfb value/copy/move/freeze semantics or the thread-transfer model; the
  Swift values live entirely inside the bridge and are marshalled at its boundary.
- No async surface exposed to MFBASIC — the worker sees a **synchronous** call that
  blocks; async lives inside the bridge (this is why the worker/main split from 30-C
  matters).

## 2. Current State (measured against the oracle)

- No Swift ABI anywhere in mfb. Precedents to mirror: hand-emitted
  `objc_msgSend`/`sel_registerName` in `src/target/macos_aarch64/app/`; the async
  **block-callback trampoline** for Network.framework in
  `src/target/macos_aarch64/tls.rs` (closest existing "framework calls us back
  asynchronously" model).
- Oracle findings (Swift 6.2, iPhoneSimulator SDK 26.2 — see this doc's history):
  - swiftcc register discipline on arm64: **x20 = swiftself**, **x21 = swifterror**
    (a `throws` callee zeroes it on entry; caller checks after the call),
    **x22 = swiftasync** (async context pointer); large/address-only results via x8.
  - ARC entry points seen: `swift_retain`, `swift_release`,
    `swift_bridgeObjectRelease`, `swift_errorRelease`; exclusivity `swift_beginAccess`.
  - **The entire concurrency runtime surface for fetch-products + purchase + observe
    is three symbols:** `swift_task_alloc`, `swift_task_dealloc`, `swift_task_switch`.
    Total external symbols for that call set: ~45.
  - Async functions are referenced as `…Tu` **async function pointer** symbols; the
    compiler splits each async fn into `…TY<n>_` (suspend-resume) / `…TQ<n>_`
    (await-resume) partial functions — we do **not** need to *split our own* code if
    the bridge is structured as one job that drives the callee and resumes via a
    continuation.
  - Generic instantiation goes through metadata accessors (`…Ma`) /
    `swift_getTypeByMangledNameInContext` / `__swift_instantiateConcreteTypeFromMangledNameV2`;
    concrete StoreKit types expose accessor symbols we reference by name.

## 3. Design Overview

Three layers, each independently oracle-validated, lowest-risk first:

1. **D1 swiftcc sync foundation** — register discipline + error-slot + ARC +
   metadata referencing, proven on a trivial concrete sync call.
2. **D2 async driver** — the one-time hard part: create a top-level task, set up an
   async context/frame, invoke a `…Tu` async function pointer, and provide a
   continuation that posts completion to a semaphore the worker blocks on; handle the
   `swift_task_switch` executor hop.
3. **D3 marshalling** — the value bridges (String, Array, enum, error) at the bridge
   boundary.

All correctness risk is in D2. The mitigation is the **oracle-diff harness**: for
each shape, the reference `.swift`'s assembly is the spec, and the phase is done only
when mfb's emitted sequence is functionally equivalent and runs on the simulator.

## 4. Detailed Design

### 4.1 D1 — swiftcc sync call

Emit a swiftcc call: place `self` in x20 (if a method), zero x21 before a `throws`
call, marshal args per Swift's aggregate-explosion rules, `bl` the `$s…` symbol,
then **check x21** — nonzero means a thrown error (route to `swift_errorRelease` +
mfb error). Insert `swift_retain`/`swift_release` at the ownership points the oracle
shows. Reference type metadata via the `…Ma` accessor when a generic must be
instantiated.

### 4.2 D2 — async driver (the spike)

- **Top-level task:** create one `Task` to host the async work (the oracle's
  `@_cdecl … Task { }` probe shows the `swift_task_create`-family call + reabstraction
  thunk shape to copy).
- **Async frame:** on entry to the driver, `swift_task_alloc` the async context /
  frame for live state across the suspension; `swift_task_dealloc` on completion.
- **Invoke the callee:** load the callee's `…Tu` async function pointer, set the
  async context register (x22), and transfer per the async convention; the callee
  resumes our **continuation** function.
- **Continuation → unblock worker:** our continuation posts to a semaphore (or writes
  the result to the inbound resource queue) that the **worker thread** — parked in
  30-C's channel — is waiting on. This is what turns async into a synchronous-looking
  worker call.
- **Executor hop:** honor `swift_task_switch` where StoreKit hops executors
  (background for the request, `@MainActor` for parts of purchase); running on the
  30-C main thread satisfies the main-actor requirement.

### 4.3 D3 — marshalling

- **mfb `String[]` → Swift `[String]`:** build Swift `String`s (bridge via the String
  ABI / `_bridgeToObjectiveC` as the oracle shows for literals) and an `Array`
  through its metadata + value witness; pass to `products(for:)`.
- **Swift `Array<Product>` → mfb list:** element access via the `Array` ABI + value
  witness; copy each element out, then release the Swift array.
- **Swift enums → mfb:** extract the case tag (`VerificationResult.verified`,
  `PurchaseResult.success`) and payload by the layout the oracle prints.
- **`throws` → mfb:** x21 error → mfb error value / `TRAP` route, balanced with
  `swift_errorRelease`.

## Layout / ABI Impact

Adds Swift-runtime imports (`swift_task_alloc`/`_dealloc`/`_switch`, ARC, metadata
accessors) to the iOS backend imports, and `libswiftCore`/`libswift_Concurrency`
(system, ABI-stable, shipped in the simulator runtime) to the dylib table (30-A).
**No mfb-visible layout, copy/transfer, or golden change** — Swift values exist only
inside the bridge; the boundary marshals to ordinary mfb values.

## Phases

### Phase 1 — D1 swiftcc sync + oracle-diff harness

- [ ] Add the oracle-diff harness: given a reference `.swift`, emit its assembly and provide a functional-equivalence check against mfb's emitted sequence (dev tooling under `scripts/` or a test util).
- [ ] Emit a swiftcc sync call (concrete, `throws`): register discipline, x21 error check, ARC, metadata accessor referencing.

Acceptance: mfb calls a trivial sync `throws` Swift function on the simulator and
returns the correct result, with balanced ARC (leak check clean) and the thrown-error
path routed to an mfb error.
Commit: —

### Phase 2 — D2 async driver (highest-risk)

- [ ] Emit the top-level task + async-frame (`swift_task_alloc`/`_dealloc`) + continuation glue; drive a trivial `async throws` Swift fn (e.g. the `fetch(Int)` probe) to completion.
- [ ] Wire the continuation to unblock the 30-C worker (semaphore / inbound queue); handle the `swift_task_switch` executor hop.

Acceptance: mfb-emitted code calls a real `async` Swift function on the simulator and
the worker thread receives the correct result after suspension/resume; ARC balanced.
Commit: —

### Phase 3 — D3 marshalling

- [ ] `String[]`→`[String]`, `Array<T>`/enum results→mfb, `throws`→mfb error, each oracle-diffed.

Acceptance: round-trip a `[String]` argument and an `Array`/enum result through the
bridge on the simulator with correct values and balanced ARC.
Commit: —

## Validation Plan

- Function tests: none directly (the bridge has no MFBASIC surface of its own —
  30-E's `iap::` tests exercise it end-to-end).
- Runtime proof: Phases 1–3 each run a real Swift call on the simulator with a
  checked result + a leak/refcount check.
- Doc sync: add a `src/docs/spec/**` topic documenting the swiftcc register
  discipline, the async-driver contract, and the "oracle, not build dependency" rule.
- Acceptance: `scripts/test-accept.sh` unaffected (no existing-target impact).

## Open Decisions

- **Continuation → worker signal** — recommend a **semaphore** the worker blocks on
  (simplest, matches `tls.rs` blocking model) vs. writing the result to the inbound
  resource queue (reuses 30-C plumbing, better if multiple in-flight). (§4.2)
- **Task hosting** — recommend **one long-lived top-level task** reused per call vs.
  a fresh `Task` per call (simpler lifetime, more churn). (§4.2)
- **ARC verification** — recommend a per-phase refcount assertion via a debug hook /
  `leaks` on the simulator process vs. eyeballing the oracle diff. (§4.1)

## Non-Goals

- Arbitrary Swift interop, generic user-defined Swift calls, or a public
  Swift-FFI surface — only the fixed StoreKit call shapes.
- `async`/`await` in the MFBASIC language.

## Summary

This is the feature's real engineering risk, and it's bounded by measurement: three
concurrency entry points, ~45 symbols, async callees referenced as named `…Tu`
symbols. The hard part (D2, the async driver) is written **once**, against the oracle,
and every other call is small glue. The "no external compiler" constraint is honored
exactly — `swiftc` is a development-time spec reader, the shipped compiler emits every
byte itself. Nothing in the mfb language or memory model changes; Swift values never
escape the bridge.
