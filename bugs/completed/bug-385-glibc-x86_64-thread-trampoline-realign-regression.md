# bug-385: glibc-x86_64 thread trampoline realign SIGSEGVs (bug-383 over-broad gate)

Status: FIXED (2026-07-25, commit <pending>)
Found: CI `coverage` job (Ubuntu x86_64 glibc) went red on a real test failure —
`thread_queue_limit_in_range_accepted` — immediately after bug-383 landed.

## Resolution

`lower_thread_trampoline` (`src/target/shared/code/runtime_helpers.rs`) now gates
the +8 trampoline realignment on `arch == "x86_64" && (family == Windows || libc
== Musl)` instead of bug-383's `arch == "x86_64"`. glibc-x86_64 takes NO realign
and keeps the byte-identical 80-byte frame; Windows and musl x86-64 keep the
88-byte frame (bug-383's real fix for musl is preserved). aarch64/riscv64/macOS
are unaffected. No golden churn — the trampoline is emitted only for threaded
modules and no byte-identity fixture is one.

Proven on the real boxes (exact repro of the failing test — imported
`thread_runtime_workers::emitThreeBuffered`, `inLimit=1`, `outLimit=3`):

- 2228 (Ubuntu x86_64 glibc): HEAD's 88-byte frame SIGSEGVs (exit 139); the fix's
  80-byte frame prints `one` (exit 0).
- 2227 (Alpine x86_64 musl): the fix's 88-byte frame prints `one` (exit 0) —
  bug-383's musl alignment is retained.

## Claim

bug-383 changed the realign gate from Windows-only to *all* x86-64 on the premise
that "on x86-64 the trampoline is always `call`-reached, so it always arrives at
`sp % 16 == 8`." That premise is **false for glibc**. glibc's `start_thread`
enters the thread start-routine already 16-aligned (`sp % 16 == 0`), so folding
+8 into the 16-multiple frame leaves every downstream call at `sp % 16 == 8` — a
16-misaligned stack. Unlike the "latent/tolerated" misalignment bug-383 assumed
for linux, this is a hard fault: the first callee that spills a `__m128`/`__m256`
to an rsp-relative local with `movaps`/`vmovaps` #GPs → SIGSEGV.

## Mechanism (why the entry alignment differs per x86-64 ABI)

The trampoline is the pthread start-routine, and how the C library reaches it
sets its entry alignment:

- **Windows x86-64**: BaseThreadInitThunk `call`s it → enters at `sp % 16 == 8` →
  needs +8 (47-H).
- **musl x86-64**: the pthread start-routine dispatch `call`s it → enters at
  `sp % 16 == 8` → needs +8 (bug-383, proven on 2227).
- **glibc x86-64**: `start_thread` enters it already 16-aligned (`sp % 16 == 0`)
  → needs NO realign. bug-383's +8 broke exactly this case.

The alignment is therefore a per-flavor property, not a per-arch one, which is
why the fix keys on `libc()` (already `Some(Glibc)`/`Some(Musl)` per codegen pass,
since a Linux build lowers once per flavor) rather than on `arch()`.

## Note on bug-383

bug-383's doc states the x86-64 "Mechanism" as if both linux flavors enter at
`sp % 16 == 8`; that is correct for musl but wrong for glibc. bug-383's musl fix
is real and retained — only its over-broad gate (and the glibc half of its
mechanism claim) is corrected here.
