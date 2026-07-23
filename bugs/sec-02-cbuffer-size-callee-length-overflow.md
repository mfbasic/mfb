# sec-02: `OUT CBuffer` allocates `SIZE` bytes but never bounds the length handed to the C callee (heap/arena overflow)

Last updated: 2026-07-23
Effort: medium (1h–2h for the mitigation; the preventive fix is a design decision)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: (to add) a runtime fixture under `tests/rt-behavior/native/` where a
`BUFFER buf SIZE <small>` binding hands the callee a larger write length and a
guard (canary/redzone) traps instead of silently corrupting the arena; plus an
`ir::verify`/lowering twin if a verifier rule is chosen.

A native `LINK` function with an `OUT CBuffer` slot allocates a byte-list buffer
of exactly the `BUFFER <slot> SIZE <expr>` capacity, hands the C callee a pointer
to that buffer's data region, and separately marshals every *other* ABI slot —
including whatever integer the callee interprets as "how many bytes may I write
into that buffer" — straight through from the wrapper parameter with **no
relationship enforced between the two**. If the declared `SIZE` is smaller than
the length value the callee receives, the C function writes past the allocated
block and corrupts adjacent arena memory (collection headers, resource records,
other live blocks). The `RETURN <slot> LENGTH <expr>` clause clamps only the
*returned view* of the buffer, and it runs *after* the native call — the overflow
has already happened.

The single correct behavior a fix produces: a `CBuffer` binding whose callee
write length exceeds the allocated `SIZE` must not silently corrupt memory. Either
(a) the thunk detects the overrun and aborts before the corrupted memory is used
(a post-call canary/redzone — universal, mechanical), and/or (b) the language
gains a way to declare which slot is the buffer's write-length so the thunk can
clamp it to `min(len, SIZE)` at runtime (preventive, covers the common
single-length-arg C APIs like `read`/`pread`/`recv`). Documentation must also
state plainly that `SIZE` is a safety boundary the author is responsible for, not
merely a capacity hint.

This is reachable **two ways**: (1) a source-level `LINK` binding in the app's own
project whose `SIZE` expression is understated relative to the length it passes
(an ordinary author arithmetic mistake — `frames * channels` where the C API
writes `frames * channels * 2`, or a hard-coded constant), and (2) a compiled,
third-party `.mfp` package that carries such a binding: the `buffers`/`SIZE`
surface now rides the wire format (plan-58-C, `src/ir/binary.rs:377-380` /
`:645-652`), decodes in the *consumer's* toolchain, and lowers into the consumer's
app. As with sec-01, the package compiles without a diagnostic, and its clean
compile is a false guarantee of memory safety for every app that links it.

References:

- `src/docs/spec/language/17_native-libraries.md:180-220` — the `CBuffer` /
  `BUFFER … SIZE` / `RETURN … LENGTH` contract. Line **195** records the
  deliberate design choice to put capacity on the author-written `SIZE`
  expression (auto-deriving it from a sibling length slot was *rejected* because a
  C function may take two lengths and the convention "silently picks the wrong
  slot" — "the clause states the relationship the C API actually has"). Line
  **216** makes `LENGTH` mandatory to avoid exposing *unwritten* bytes; line
  **220** bounds `SIZE` itself against a ceiling. Nothing in the contract bounds
  the callee's *write length* against `SIZE`.
- The canonical safe example (spec `:185-187`): `BUFFER buf SIZE frames*channels*2`
  is memory-safe **only because** `SIZE` derives from the same `frames` parameter
  the callee uses to decide how much it writes. That coupling is author
  discipline, enforced nowhere.
- Memory: `[[imported-package-resource-two-spellings]]`, `[[link-thunk-never-reclaims-the-record]]` (the FFI/resource trust surface); sibling finding
  `bugs/sec-01-link-free-resource-uaf.md` (same "crafted binding → consumer
  memory-unsafety" threat model; distinct mechanism — that one is `FREE`+`AS RES`
  UAF/double-free, this one is an OUT-buffer write overflow).
- Found during the 2026-07-23 runtime security audit (FFI marshaling sweep).

## Failing Reproduction

A source-level binding accepted by `syntaxcheck`, `ir::verify`, and lowering
today. It binds only the trusted, auditable libc symbol `pread` — the memory
unsafety lives entirely in the ABI declaration:

```
LINK "c" AS libc
  FUNC preadBytes(fd AS Integer, nbyte AS Integer, offset AS Integer) AS List OF Byte
    SYMBOL "pread"
    ABI (fd CInt32, buf OUT CBuffer, nbyte CInt64, offset CInt64) AS got CInt64
    BUFFER buf SIZE 16          ' allocate only 16 bytes ...
    RETURN buf LENGTH got       ' ... but the nbyte slot still carries the caller's value
  END FUNC
END LINK

FUNC main() AS Integer
  ' Open some file into fd (elided) ...
  LET bytes = libc::preadBytes(fd, 100000, 0)  ' pread(fd, buf16, 100000, 0)
  RETURN 0
END FUNC
```

- Observed: compiles without diagnostic. The thunk allocates a 16-byte arena
  byte-list, passes its data pointer as `buf`, and passes the caller's `100000`
  verbatim into `pread(fd, buf, 100000, 0)`. The real libc `pread` writes up to
  100000 bytes into the 16-byte block, overrunning it by ~99 984 bytes into
  adjacent arena memory. `got` is clamped to `[0,16]` on the way out — but the
  corruption already occurred during the call.
- Expected: the overrun is trapped (guard/redzone) before any corrupted memory is
  read, or the callee's write length is clamped to `16` so `pread` cannot exceed
  the allocation; at minimum the hazard is a documented, testable contract rather
  than a silent heap smash.

Contrast case that is correct today and must stay accepted (spec `:185-187`,
`tests/rt-behavior/native/native-cbuffer-read-rt`): `BUFFER buf SIZE nbyte` with
`RETURN buf LENGTH got`. Here `SIZE` reads the very `nbyte` parameter the callee
uses as its count, so the allocation is always ≥ what the callee can write. The
binding is safe **only** because the author coupled the two by hand.

Imported-package variant: the same declaration compiled into a `.mfp` and imported
by a second project. `buffers`/`SIZE` decode via
`src/ir/binary.rs:645-652` (`decode_vec_capped(r, MAX_LINK_BUFFERS, …)`), the
allocation ceiling applied is the *consumer's* `maxBuffer` (deliberately not
carried in the package — `src/ir/binary.rs:369-372`), and `SIZE 16` clears any
sane ceiling trivially. The consumer's app overflows with no source-level review
of the binding.

## Root Cause

The buffer allocation and the length marshaling are emitted independently, and
neither the verifier nor codegen relates them.

Codegen, `src/target/shared/code/link_thunk.rs`:

- `:759-767` — `SIZE` is evaluated (`emit_link_expr`) into `size_reg` and spilled.
- `:774-780` — the *only* bound on the size: `size < 0 → fail`, `size >
  max_buffer_bytes → fail`. This bounds the **allocation**, not the callee write.
- `:787-795` — `emit_alloc_byte_list` allocates a block whose data capacity is
  exactly `SIZE`.
- `:802-806` — `dataBase` (block + `COLLECTION_HEADER_SIZE`) is stored into the
  buffer slot's cslot; this pointer is what the C function receives as `buf`.
- `:915-960` — every other ABI slot is marshaled from its wrapper parameter.
  A `CInt64` length slot (`nbyte`) falls to the generic arm `:955-959`
  (`load param; store cslot`) and reaches the callee **verbatim**. Nothing here,
  or anywhere, compares it to `size_reg`/`SIZE`.
- `:1243-1266` — the post-call path clamps `RETURN … LENGTH` to `[0, capacity]`.
  This sanitizes the *result list length* only, and runs after the native call —
  it cannot prevent the write overrun.

The compiler fundamentally cannot know, from the ABI line alone, which integer
slot the C callee treats as "the length of `buf`" (spec `:195` is explicit: a C
function may take two lengths). So the safety of the write rests entirely on the
author's `SIZE` expression being ≥ the callee's maximum write — an invariant the
toolchain never checks.

Missing guards:

- `src/ir/link.rs:549-732` (`check_buffer_slots`, rules 1–11) constrains
  direction (OUT-only), clause count (exactly one `SIZE`), `RETURN` naming,
  `LENGTH` presence, and *what a `SIZE` expression may read* (causality — params
  and `CONST` pins only, `:670-695`). No rule relates `SIZE` to any slot the
  wrapper feeds the callee as a length.
- `src/ir/verify/link.rs` — re-runs `check_buffer_slots` on the decoded package
  (load-bearing: a `.mfp` bypasses `syntaxcheck`) but inherits the same gap.

## Goal

- A `CBuffer` binding whose callee write length exceeds the allocated `SIZE`
  cannot silently corrupt the arena: the overrun is trapped before any corrupted
  memory is used, and/or the callee's length is clamped to the allocation.
- The correct coupled bindings (`native-cbuffer-read-rt`, the libsndfile
  `sf_readf_short` shape) still compile and run byte-for-byte as today.

### Non-goals (must NOT change)

- The `CBuffer` ABI, the `SIZE`/`LENGTH` clause syntax, the `[0, capacity]` result
  clamp, or the buffer-ceiling check — all correct and must stay.
- The rejection of auto-deriving capacity from a sibling slot by naming
  convention (spec `:195`) — that decision stands; a fix must be explicit, not a
  silent heuristic that "picks the wrong slot."
- Do NOT weaken `native-cbuffer-read-rt` / `native-cbuffer-valid` — they exercise
  the *correct* coupled path.

## Blast Radius

- `link_thunk.rs:759-806, 915-960, 1243-1266` — the OUT-CBuffer allocate +
  length-marshal + post-clamp sites; where a guard/clamp would land.
- `src/ir/link.rs:check_buffer_slots` and `src/ir/verify/link.rs` — where any
  new verifier rule (if chosen) must be mirrored so a `.mfp` cannot smuggle the
  mismatch past the source-level check.
- Every in-tree `OUT CBuffer` binding — all currently couple `SIZE` to the
  callee's length by hand (audio `sf_readf_short`: `SIZE frames*channels*2` with
  `frames` passed; the `pread`/`read` fixtures: `SIZE nbyte` with `nbyte`
  passed). None is *observed* to overflow, but every one is one arithmetic slip
  away — that is the latent hazard this closes.
- Non-`CBuffer` OUT paths (scalar OUT, `CSTRUCT` OUT) — unaffected: their sizes
  are compile-time constants the callee cannot be told to exceed (the struct
  buffer is fully zeroed and sized by `SIZEOF`, spec `:119,:136`).

## Fix Design

Two complementary directions; (A) is a universal mechanical mitigation, (B) a
preventive for the common case. Recommend landing (A) first (it covers every
shape), then evaluating (B).

- **(A) Post-call redzone / canary (universal, mechanical).** Allocate `SIZE` plus
  a small guard region, write a canary immediately past the `SIZE`-th byte before
  the call, and after the call compare it; a mismatch traps with a clear runtime
  error (`ErrNativeBufferOverrun`) instead of continuing on corrupted memory.
  This does not *prevent* the overrun but converts a silent, exploitable
  heap-smash into a deterministic abort — the same posture as an ordinary
  bounds-check trap. Cost: a few bytes per buffer and one compare per `CBuffer`
  call. Applies to every C API shape, including multi-arg write-size derivations.
- **(B) Declared length slot + runtime clamp (preventive, common case).** Add an
  optional clause naming the ABI slot the callee treats as `buf`'s write length
  (e.g. `BUFFER buf SIZE <expr> LIMIT <slot>`); the thunk then marshals that slot
  as `min(value, SIZE)`, so `read`/`pread`/`recv`-style APIs physically cannot
  exceed the allocation. Explicit (satisfies spec `:195`), opt-in, and leaves the
  multi-length derivations to (A).
- Documentation: state in `src/docs/spec/language/17_native-libraries.md` and the
  `link` man page that `SIZE` is a memory-safety boundary the author owns — the
  callee must never be handed a write length exceeding it.

Rejected alternative — inferring the length slot by position/type/name
convention: explicitly rejected by the existing design (spec `:195`) and unsafe
for C functions taking two lengths.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a runtime fixture: a `BUFFER buf SIZE <small>` binding that passes a
      larger callee length; confirm it corrupts/aborts today (ASAN or a poisoned
      guard block makes the overrun observable). This is the failing state.
- [ ] Audit every in-tree `OUT CBuffer` binding; record for each how `SIZE`
      couples (or fails to) to the callee's write length. Confirm none is
      *currently* understated (they are safe by author discipline, not by check).

Acceptance: the fixture demonstrably overruns today; the audit has a verdict per
binding.
Commit: —

### Phase 2 — the mitigation

- [ ] Implement (A) in `link_thunk.rs`: guard region + post-call canary check →
      `ErrNativeBufferOverrun`.
- [ ] (If pursuing (B)) parser + `check_buffer_slots` + `ir::verify` + wire format
      + thunk clamp for the `LIMIT` slot.

Acceptance: the Phase 1 fixture traps (or is clamped) instead of corrupting;
the coupled fixtures still pass unchanged; no golden output moves for existing
valid bindings.
Commit: —

### Phase 3 — validation

- [ ] Run the native-link runtime suite + `ir::verify` suite.
- [ ] Confirm no `.mfp`/golden deltas for existing valid packages.

Acceptance: full suite green; the only new behavior is the trap/clamp on an
understated `SIZE`.
Commit: —

## Validation Plan

- Regression test(s): the new understated-`SIZE` runtime fixture (overrun →
  trap/clamp) plus, if (B) lands, an `ir::verify` twin for the decoded package.
- Runtime proof: the coupled `native-cbuffer-read-rt` still reads correctly;
  the understated binding aborts deterministically instead of smashing the arena.
- Doc sync: `17_native-libraries.md` + `link` man page note that `SIZE` is a
  safety boundary.
- Full suite: the project's native-link + `ir::verify` acceptance gates.

## Open Decisions

- Mitigation scope — (A) canary only (recommended first; universal) vs. (A)+(B)
  clamp (prevents the common single-length case outright). (§Fix Design)
- Severity framing — impact-if-reached is a controlled heap overflow (HIGH), but
  unlike sec-01 the binding is internally consistent from the compiler's view (the
  "wrongness" is only against the C library's real behavior, which the compiler
  cannot see), so there is no zero-false-positive static rejection — the fix is a
  runtime guard, not a frontend reject.

## Summary

`OUT CBuffer` marshaling allocates exactly `SIZE` bytes and hands the C callee a
pointer to them, but the length the callee uses to decide how much it writes is a
separate ABI slot passed through verbatim, never bounded against `SIZE`. The
buffer's memory safety therefore rests entirely on the author writing a `SIZE`
expression ≥ the callee's maximum write — an invariant neither `syntaxcheck`,
`ir::verify`, nor codegen checks. An understated `SIZE` (an arithmetic slip, or a
hostile/careless `.mfp` package that now rides the wire format) compiles clean and
overruns the arena at runtime. The robust fix is a post-call canary that traps the
overrun before corrupted memory is used, optionally plus a declared length slot
the thunk clamps to `SIZE`; the correct coupled bindings are unaffected.
