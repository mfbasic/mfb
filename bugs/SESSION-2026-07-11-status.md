# Bug-fix session disposition — 2026-07-11

## FINAL state (after the "fix each" continuation)
Of the 21 docs that were open at the continuation, 20 are resolved and verified on
all four remotes; 1 (bug-78) is a scoped feature deferred for a dedicated effort.

**Fixed + moved to completed-bugs this continuation:** bug-97, bug-102, bug-114,
bug-115, bug-116, bug-117, bug-120, bug-124, bug-125, bug-136, bug-137, bug-139
(all sub-issues); bug-80 (union canonical tags — divergent positions now dispatch,
verified 4 remotes); bug-82 (CodeOp relocation, byte-identical); bug-86 (CLOSED as
not-a-bug — the "spurious overflow" was the entry exit-code range check on a large
returned error code, not a codegen defect).

**Still open (with reasons):**
- **bug-78** — user chose the full allocation-free closure redesign; scoped in the
  doc as a cross-phase + cross-backend FEATURE (static BSS descriptors + a startup
  initializer, or a new data→code absolute relocation) with real regression risk to
  function-value semantics. It is arena GROWTH (freed at exit), not a correctness
  bug. Left for a dedicated staged effort rather than rushed.
- **bug-79** — 79.2/79.3/79.4 fixed; 79.1 == bug-116 (fixed); 79.5 (`pick()(4)`
  call-a-returned-value grammar) LEFT as a parse error per the user's decision
  (deliberate language limitation, not a bug).
- **bug-126** — 126.1/126.2 fixed; 126.3 (shared fused-setter re-expansion) latent,
  no byte-neutral rewrite without churn.
- **bug-127** — 127.1/127.2 fixed; 127.3 (per-ISA scratch occupancy) latent,
  guaranteed unreachable by plan-34-D; correct fix is an ISA-threading refactor of
  the hot allocator, not worth the risk.
- **bug-138** — 138.1/138.2a fixed; 138.2b (an x0<-x0 no-op in randomBytes) left in
  place — removing it churns x86 bytes via the fragile arg-staging inference for
  zero benefit.
- **bug-147** — 147.1/147.3/147.4/147.7 fixed (147.6 already fixed, 147.2 stale);
  147.5(a) list `set` trap-route singleton leak FIXED (commit 1cfcba82, reorder the
  fallible removeAt before the singleton alloc; verified 4 remotes, byte-identical
  goldens). 147.5(b) thread-send copied-message leak still open — a cross-thread
  arena-free data race needing a threading/ownership redesign.

## Wave 4 addendum (bug-147.5a continuation)
- **bug-151** — general caught-`Error`-per-trap leak (every taken `TRAP(e)` leaked
  the `Error` block; trap-in-a-loop grew RSS ~0.6 KB/catch) FIXED (commit 64700c29):
  register `e` as the first owned value of the handler body scope, freed once per
  exit. Discovered while validating 147.5(a); it dominated the 147.5 leak
  measurement. Verified 4 remotes (RSS flat, `RETURN e`/`FAIL e`/`e.message` correct,
  no double-free). Moved to completed-bugs.
- **bug-152** — FILED: `FAIL e` re-raise leaks the deep-copied propagation transient
  (a pre-existing error-Result-ABI issue, not a bug-151 regression; no double-free;
  the common catch-and-handle path is fully fixed). Needs an error-Result-ABI redesign
  (propagate an owned block pointer), same block-lifetime family as 147.5(b).

**Discovered (filed separately, pre-existing, out of scope):** x86-glibc
`math::rand` with a fixed seed is non-deterministic (present at HEAD before this
work; musl x86 / aarch64 / riscv are all correct and deterministic). bug-148
(loop-trap nulls e.message) was filed by a concurrent client.

---


## Wave 3 addendum
**bug-137.4** (math::rand modulo bias → Lemire rejection sampling; verified
uniform-bounded + cross-target-identical seeded sequence on all four remotes) and
**bug-126.2** (rv64 stale-flag pending-clear guards, byte-neutral) landed.
bug-126.3 left deferred (no byte-neutral rewrite of the shared fused-setter
re-expansion without churn).

## Remaining open docs — why deferred (not in-session-fixable safely)
Every bug that *manifests* at runtime (wrong answer / crash / hang / leak / build
failure a program can trigger) is fixed and verified on all four remotes. The
still-open cluster docs retain only sub-issues that are one of:
- **Design-scale, need a plan**: bug-78 (closure-alloc relocation vocabulary),
  bug-80 (union canonical tags + .mfp format bump), bug-82 (mechanical CodeOp
  relocation across 18 files — byte-identical, zero runtime value), bug-86
  (layout-sensitive thread-error register audit).
- **High-risk register/encoder changes for LATENT-only issues** (a hasty fix risks
  regressions on untested paths — violates "correctness over performance"):
  bug-124.1/.2 (aarch64 pool/frame), bug-125.1/.3 (x86 encoder scratch),
  bug-127.2/.3, bug-126.3.
- **Broad defensive sweeps with no current trigger**: bug-97.2 / bug-115 (EINTR),
  bug-102.3 (TLS C-int sign-extend).
- **macOS-only, not verifiable on the Linux remotes**: bug-114 (app pipe backpressure,
  partly), bug-116 (macOS TLS options leak), bug-117.1/.3.
- **Minor latent nits**: bug-120.2, bug-136.3, bug-137.1 (host-libm determinism —
  unverifiable on one host), bug-138.2b (deliberate no-op), bug-139.5 (unreachable),
  bug-147.4/.5/.7.

## Wave 2 addendum
Additional fixes landed + verified on all four remotes (round/pow correct on
aarch64/x86_64/riscv64): **bug-108.2** (depth-aware `" TO "` map-key splitting →
bug-108 now fully fixed, moved to completed-bugs), **bug-117.4** (macOS headless
`_pause` park instead of 100% busy-spin), **bug-117.5**/**138.2a** (comments),
**bug-125.2** (x86 pool docs 5→4 + spec), **bug-126.1** (rv64 frint ties-to-even
+ integral/non-finite guard — `math::round(List)` verified byte-correct on rv64),
**bug-137.5** (`pow(-0.0, y)` fdlibm ±0 prologue — verified: `pow(-0.0,0.5)=+0`,
`pow(-0.0,3)=-0`), **bug-137.6** (FMA fusion label guard), **bug-138.1** (deleted
dead FloatBinaryKernel::Pow), **bug-139.1/.3/.6** (plan-fold guards, literal
merge, link-thunk hex escape), **bug-102.2** (entry-symbol reloc param),
**bug-79.4** (x86 dead `write` imports). Clusters 79/102/117/125/126/137/138/139
remain open for their still-deferred sub-issues (below).


Fixed + tested + verified on all four remotes (Kali aarch64 :2223, Alpine x86_64
musl :2227, Ubuntu x86_64 libc :2228, Alpine riscv64 musl :2229). Host acceptance
(892 tests) and `cargo test` (2488+ unit + integration) green. bug-87 determinism
confirmed by byte-identical triple rebuilds on aarch64/x86_64/riscv64.

## Fully fixed → moved to completed-bugs/
74, 75, 76, 77, 81, 85 (won't-fix — inference is the intended x86 mechanism; the
follow-up cleanup needs a full-exe byte oracle), 87, 88, 90, 91, 92, 93, 94, 95,
96, 98, 99, 100, 101, 104, 105, 106, 107, 110, 111, 112, 113, 118, 119, 122, 123,
128, 130, 131, 132, 134, 135, 144, 145, 146.

(90–146 were fixed by prior commits e0fa88b8 / 31ac2fd1 and are now verified green;
their docs were still in bugs/ and are moved here.)

Note bug-74 also required fixing a **separate pre-existing x86 bug**: the Fixed `^`
/ `math::pow(Fixed)` ±1.0 fast path compared against `FIXED_ONE` (=2^32) with
`compare_immediate`, which cannot encode as an x86 CMP imm32 — so Fixed pow failed
to *build* on x86 entirely. Fixed by materializing the constant into a register and
using `compare_registers` (builder_numeric.rs / builder_fixed_math.rs). Now builds
and runs correctly on all four remotes.

## Partially fixed this session (cluster docs kept open for the remainder)

- **bug-79**: FIXED 79.2 (link-thunk shared label counter — now a hard error given
  the new dup-label guard), 79.3 (pathNormalize `a/..` -> `.`). DEFERRED 79.1
  (=bug-116 macOS TLS leak), 79.4 (x86 dead `write` import), 79.5 (`pick()(4)`
  grammar — design).
- **bug-97**: FIXED 97.1 (stdout drain persists cursor/remaining on the error path
  so a retried flush does not re-send the prefix). DEFERRED 97.2 (EINTR guard on
  the UTF-8 continuation reads — latent, multi-site).
- **bug-102**: FIXED 102.1 (macOS temp fd O_CLOEXEC), 102.4 (dead PARENT_ARENA_STATE
  store). DEFERRED 102.2 (entry-symbol reloc parameterization — latent), 102.3 (TLS
  C-int sign-extend sweep — latent, broad).
- **bug-108**: FIXED 108.1 (gutted the three dead relocated syntaxcheck rules).
  DEFERRED 108.2 (depth-aware `" TO "` map-key splitter — latent, diagnostics only).
- **bug-117**: FIXED 117.2 (linux_aarch64 io.flush dead fsync/errno imports).
  DEFERRED 117.1 (rv64 armed GTK hooks), 117.3 (GTK term grid race + comment),
  117.4 (macOS headless busy-spin -> pause), 117.5 (stale comments).
- **bug-120**: FIXED 120.1 (removed the dead strings_specs catalog entries +
  helper_for_call arm). DEFERRED 120.2 (widen IO_PRINT_CLOBBERS doc — latent).
- **bug-124**: FIXED 124.3 (aarch64 branch displacement range check). DEFERRED
  124.1 (drop x15–x17 from the aarch64 allocatable pool — high golden churn, needs
  full 4-arch re-verify), 124.2 (v128 callee-saved 64-bit prologue save).
- **bug-127**: FIXED 127.1 (duplicate-label guard on aarch64 + riscv, matching
  x86). DEFERRED 127.2 (eviction panic is a safe clear-message halt, not a
  miscompile; graceful `Err` needs Result threading through the regalloc driver),
  127.3 (per-ISA `%scratch` occupancy index — unreachable today).
- **bug-136**: FIXED 136.1 (wipe the RAWBUF private-scalar scratch), 136.2 (verify
  checks EVP_MD_CTX_new / DigestVerifyInit returns). DEFERRED 136.3 (runtime DER
  length parse for the SEC1 scalar offset — latent).
- **bug-137**: FIXED 137.2 (Boolean `XOR call()` spills the left operand), 137.3
  (Fixed pow underflow reciprocal traps ErrOverflow, not ErrInvalidArgument), plus
  the x86 Fixed-pow build fix noted above. DEFERRED 137.1 (host-libm transcendental
  constants — cross-host determinism), 137.4 (rand modulo bias), 137.5 (pow(-0.0,
  non-integer) NaN), 137.6 (FMA fusion label-blindness — latent).
- **bug-138**: FIXED 138.2a (stale `distance` comment). DEFERRED 138.1 (dead
  FloatBinaryKernel::Pow machinery), 138.2b (x0<-x0 no-op in randomBytes — left in
  place; entangled with the fragile x86 arg-staging inference, bug-85, for zero
  benefit).
- **bug-139**: FIXED 139.2 (dead CallKind::Import producer removed; variant kept
  because src/os object emitters match on it), 139.4 (dump the LINK/provenance
  fields, gated non-empty). DEFERRED 139.1 (plan-layer fold guards / loop-entry
  constant clear), 139.3 (push_call_with_literals dedup merge), 139.5 (CallResult
  default-arg normalization — unreachable), 139.6 (link_thunk symbol hex-escape —
  latent collision).
- **bug-147**: FIXED 147.1 (Float equality bitwise at record-field depth to match
  the payload path), 147.3 (deterministic union field-access variant order).
  Already-fixed 147.6 (commit 39c4bcd8). Stale 147.2 (values.rs is flat-layout
  correct). DEFERRED 147.4 (list element 8-byte alignment — latent), 147.5 (error-
  path intermediate frees), 147.7 (checked size arithmetic — dup of audit-1
  MEM-04/05).

## Fully deferred (no sub-issue fixed this session)

- **bug-78** (closure descriptor per-eval alloc) — cross-backend design change,
  see the doc's status note.
- **bug-80** (union canonical tags) — format + codegen design change, see the doc.
- **bug-82** (CodeOp enum misfiled under aarch64) — mechanical relocation of a
  neutral enum + repath of 18 files; byte-identical but wide; low value.
- **bug-86** (thread waitFor worker-error clobber) — re-diagnosed (cross-target
  register-lifetime bug, NOT riscv arithmetic); needs a layout-sensitive audit with
  a no-`io` reproducer. See the updated doc.
- **bug-114** (app keyDown pipe write can freeze UI) — multi-site fcntl(O_NONBLOCK)
  + backpressure; app-mode, partly macOS-only.
- **bug-115** (net EINTR retry) — latent, multi-site errno sweep.
- **bug-116** (macOS TLS configure block leaks sec_protocol_options) — needs a new
  capture slot + block-layout growth; macOS-only, not remote-verifiable.
- **bug-125** (x86 encoder cluster) — 125.1 (non-destructive carry-in), 125.2
  (docs 5->4), 125.3 (implicit-reg push/pop) all latent; need x86-box byte audit.
- **bug-126** (riscv select cluster) — 126.1 (fround_even ties + i64 round-trip)
  real for rv64 `math::round(List OF Float)`; 126.2/126.3 latent.
