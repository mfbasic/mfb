# bug-360: on aarch64, resource-using programs print correct output and then SIGSEGV at teardown

Last updated: 2026-07-19
Effort: medium
Severity: HIGH (every affected program exits 139 instead of 0, after doing its work correctly)
Class: Runtime / platform

Status: Fixed (2026-07-19)
Regression Test:
- `target::linux_common::code::tests::temp_directory_scratch_stays_inside_the_reserved_window`
  — pins the platform hook's scratch offsets to the window the shared helper reserves.
- `codegen_utils::assert_stack_accesses_fit_frame` — a debug-only guard in
  `finalize_frame` that fails *any* `sp`-relative body access escaping the frame
  it just sized, for every helper on every target. This is the class-level fix;
  the unit test is the bug-specific pin.
- The eleven fixtures below are the behavioral proof, run on real hardware by
  `scripts/linux-runtime-proof.sh`. They already reproduced it — they were simply
  never executed on this platform until now.

## Diagnosis (proven)

**It is not resource teardown, and it is not the ISA.** It is
`fs::tempDirectory` scribbling on its caller's saved link register, and aarch64
is merely the frame layout on which the scribble lands somewhere fatal.

`CodegenPlatform::emit_temp_directory` (Linux) parks the caller's buffer pointer
and capacity on the stack across its `getenv` call, at hard-coded offsets
`sp + 24` and `sp + 32`. Those constants date to `55066dd1c` ("Add Linux glibc
and musl native linking"), when the helper still built its own frame and they
were real. plan-00-G later moved the helper to vreg allocation, so
`finalize_vreg_body` builds the frame now — and `lower_fs_temp_directory_helper`
asked it for **zero** bytes of locals. Nothing reserved the window, and nothing
checked.

On `linux-aarch64` the resulting frame is 48 bytes; frame finalization shifts
body accesses past the 16-byte callee-saved area, so the two stores emit as:

```
4210: sub  sp, sp, #0x30        ; 48-byte frame
4214: str  x30, [sp]            ; this function's saved LR
...
4240: str  x0, [sp, #0x28]      ; buffer   — 40, inside the frame, harmless
4244: str  x1, [sp, #0x30]      ; capacity — 48 == frame top: OUT OF FRAME
```

`sp + 48` is the *caller's* `sp + 0`, and every function in this backend saves
its link register at `sp + 0`. So the store lands on the caller's saved `x30`
and the value written is `TEMP_CAPACITY` — 4096. **That is the `0x1000` in the
crash signature.** It was never a code address at all; the "wild branch to page
1" is a `ret` to the literal capacity constant.

Everything else in the report follows from that:

- *Correct output, then a crash.* The corruption is to the caller's **return
  address**, so the program keeps running correctly to the end of that caller's
  body and only dies when it returns. All output is already flushed.
- *macOS aarch64 unaffected.* Its `emit_temp_directory` calls `confstr` and
  touches no stack at all — which is why the same ISA passes there, and why the
  "look at the aarch64 encoder" hint in the original next-steps was a dead end.

### The ISA conclusion was wrong — this corrupted riscv64 too

This document's central claim was "the variable is the **ISA**," and the
three-aarch64-boxes-agree evidence behind it is real. The conclusion drawn from
it is not. Comparing the pre-fix helper across targets:

| target | helper frame | scratch store | in frame? | what sits at the caller's `sp + 0` |
| --- | --- | --- | --- | --- |
| linux-aarch64 | 48 | `sp + 48` | **no** | saved `lr` → `ret` to 4096, SIGSEGV |
| linux-riscv64 | 48 | `sp + 48` | **no** | saved `s1` (its `ra` is at `sp + 8`) |
| linux-x86_64 | 72 | `sp + 64` | yes (72 exactly) | — |

riscv64 wrote out of frame *identically* to aarch64. It survived only because
its register allocator happened to need one extra callee-saved register (`s1`)
in the calling function, which pushed the saved return address to `sp + 8` and
put `s1` in the line of fire instead. That is a per-function allocation
coincidence, not a property of the ISA — the same source on the same target
would crash if the allocator's pressure changed.

So the real severity is higher than reported: **`fs::tempDirectory` silently
corrupted a callee-saved register in its caller on riscv64 on every call**, and
that never surfaced as a test failure. Only x86-64 was genuinely clean, and only
by the accident of a 72-byte frame ending exactly at the store. Three aarch64
boxes agreeing was sound evidence of *something*; it was not evidence that the
ISA was the cause, and it steered the investigation at the aarch64 encoder,
which was never involved.
- *Every confirmed fixture.* All eleven reach `fs::tempDirectory`, nine of them
  through `fs::createTempFile`. The `RESOURCE` correlation was an artifact of
  how those fixtures obtain a `File`, not a property of resource lowering.

### Fix

- `emit_temp_directory` (Linux) uses `sp + 0` / `sp + 8`.
- `lower_fs_temp_directory_helper` reserves `TEMP_DIRECTORY_SCRATCH_BYTES` (16)
  via `finalize_vreg_body_with_locals`, and the trait method documents that
  window as the only stack the hook may address.
- `finalize_frame` now asserts (debug builds) that no `sp`-relative body access
  escapes the frame, so the next such drift fails at the compiler rather than
  1,500 lines of output later on one ISA.

### Result

`scripts/linux-runtime-proof.sh` on all three aarch64 boxes: **467 passed / 0
failed**, up from 446 / 21, with the three verdict lists diffing empty against
each other. All 17 aarch64-specific failures are gone.

The fix itself accounts for 17 of the 21. The remaining 4 were **defects in the
proof harness**, confirmed and fixed rather than left as suspicions (see below).

### The last 4 were harness bugs, now fixed

This document originally called them "most likely artifacts of
`scripts/linux-runtime-proof.sh`" and said to confirm before believing it. That
confirmation was done; both causes were real, and both are repaired in the
harness. Neither was ever a product defect.

**1 — no `target/` directory on the box.** 78 fixtures write scratch files to a
cwd-relative `target/` (`fs::writeText("target/bug159_regfile", …)`). Locally
that directory always exists, because it is cargo's build directory, so
`test-accept.sh` never had to create it. The harness ships only `tests/`, so
every such write failed and the fixture reported a "not found" that read exactly
like a product regression. This is the same class of error as the `cd
$REMOTE/root` bug already documented in the script's own comments. Fixed by
creating `target/` in the shipped root. One shared directory is correct — that is
what the real harness has — and is concurrency-safe at any `JOBS`: no `target/`
path is written by more than one fixture, verified across all 78.
Accounts for `csv/csv-behavior`, `fs/file-buffered-drain-integrity-rt`, and
`fs/bug159_listdir_notdir_error`.

**2 — wrong `argv[0]`.** `test-accept.sh` runs the executable by its
repo-root-relative path (`tests/<rel>/build/<name>.out`), so that string is what
a program reading `args` sees as `argv[0]`, and it is what the golden records.
The harness ran the same bytes from `/tmp/<pid>-<name>`, so `argv[0]` differed
and the compare failed on output that was otherwise correct. Fixed by landing
the executable at the path the golden names and invoking it there — bare
relative, no `./`, since a `./` would itself land in `argv[0]`.
Accounts for `project/project-entry-args-runtime`.

The harness is now a faithful stand-in for `test-accept.sh` on all three axes it
had drifted on: cwd, scratch directory, and `argv[0]`.

### Verification

| check | result |
| --- | --- |
| 2222 Arch aarch64/glibc 2.35 | **467 pass / 0 fail** (was 446 / 21) |
| 2223 Kali aarch64/glibc 2.42 | **467 pass / 0 fail** (was 446 / 21) |
| 2224 Alpine aarch64/musl | **467 pass / 0 fail** (was 446 / 21) |
| all three aarch64 verdict lists | byte-identical to each other |
| 2227 Alpine x86_64/musl | re-run in progress; its own pre-existing set (`json/json-behavior`, `os::userName` on Alpine, the two `listdir-order` fixtures whose goldens encode one directory order) is unrelated to these two harness bugs and is triaged separately below |
| macOS `scripts/test-accept.sh` | 1014 / 1014 |
| `cargo test` | 3096 passed, 0 failed |

**Blast radius, all three Linux targets.** `scripts/linux-artifact-baseline.sh`
captured 11,154 artifact hashes before and after. Exactly **19 of 1014 fixtures**
changed a single byte, and every one of them reaches `fs::tempDirectory`: the
eleven confirmed fixtures above, the three `createTempFile` fixtures, the three
`func_fs_{flush,isBuffered,setBuffered}_valid` fixtures (which obtain their
`File` from `createTempFile` — which is why they were in the failing set and why
the report could not place them), `http/func_http_respondPath_valid`, and
`syntax/fs/func_fs_tempDirectory_valid`.

The other 995 fixtures are byte-identical on `linux-aarch64`, `linux-x86_64`,
and `linux-riscv64`, so no fixture outside that list can have changed verdict on
any box — which is what closes out riscv64 and x86-64 without a second run on
emulated hardware.

Found while validating bug-321 (which is a pure reorganization and is **not** the
cause — see Not Caused By below). All ten `rt-behavior/resources/*` fixtures that
these boxes run, plus `rt-behavior/trap/trap-function-inline-errors-rt`, run to
completion on `linux-aarch64`, print byte-correct output, and then die with
`SIGSEGV` (exit 139) during teardown. All eleven **pass** on `linux-riscv64`/musl
and `linux-x86_64`/musl, from the same sources.

**This is an aarch64 bug, not a libc bug.** Both hypotheses this document
entertained on the way here were wrong, and the record is kept deliberately so
neither gets re-derived:

1. *"glibc 2.42 regression"* — killed when 2222 (glibc **2.35**) segfaulted on the
   same binary.
2. *"aarch64 + glibc pairing"* — killed when 2224 (Alpine aarch64, **musl**)
   segfaulted too. Every non-aarch64 box passes; every aarch64 box fails,
   regardless of libc. The variable is the **ISA**.

**Not a glibc-version regression.** The first draft of this document guessed it
was 2.42-specific because 2223 (Kali, glibc 2.42) was the only box up. When 2222
(Arch Linux ARM, **glibc 2.35**) came back, the *same binary* — sha256
`ed640ac9…3ea8c2` — segfaulted there identically. Seven years of glibc apart, same
crash; the version hypothesis is dead and should not be re-derived.

Because the program's own output is correct and complete, nothing upstream of
process exit is wrong; the fault is in the shutdown path (scope-drop / resource
reclamation / arena unmap / `_mfb_shutdown`).

## Crash signature

From `coredumpctl` on 2222 (which, unlike 2223, has cores enabled):

```
Signal: 11 (SEGV)
Stack trace of thread 401:
#0  0x0000000000001000 n/a (n/a + 0x0)
#1  0x0000000000001000 n/a (n/a + 0x0)
```

The PC is **`0x1000`** — page 1, never a mapped code address here — and there are
no recoverable frames. This is a *wild branch*, not a bad dereference: control
transferred through a garbage/uninitialized function pointer or a clobbered link
register. That it happens only after all program output is flushed puts it in the
teardown path.

## Reproduction

Box 2222 (Arch Linux ARM, `ldd (GNU libc) 2.35`) and box 2223 (Kali,
`ldd (Debian GLIBC 2.42-16) 2.42`), both aarch64 — identical behavior:

```
$ cp -R tests/rt-behavior/resources/resource-state-valid /tmp/rsv && rm -rf /tmp/rsv/build
$ mfb build -q --target linux-aarch64 /tmp/rsv
$ scp -P 2223 /tmp/rsv/build/*-glibc.out test@127.0.0.1:/tmp/rsv.out
$ ssh -p 2223 test@127.0.0.1 '/tmp/rsv.out; echo "[exit $?]"'
stated
[exit 139]
Segmentation fault
```

Golden (`tests/rt-behavior/resources/resource-state-valid/golden/build.log`)
expects exactly `stated` then `[exit 0]`. The output matches; only the exit
status does not.

## Affected fixtures

Confirmed failing on both aarch64/glibc boxes, all of which **pass** on
riscv64/musl and x86_64/musl — which is what rules out the resource feature
itself being broken:

```
rt-behavior/resources/bug141_resource_union_return
rt-behavior/resources/bug246_res_bind_error_plain_trap
rt-behavior/resources/inline-trap-collection-escape-rt
rt-behavior/resources/resource-borrow-across-ops-valid
rt-behavior/resources/resource-res-binding-valid
rt-behavior/resources/resource-return-collection-order-rt
rt-behavior/resources/resource-state-mutation-valid
rt-behavior/resources/resource-state-valid
rt-behavior/resources/resource-union-foreach-valid
rt-behavior/resources/resource-union-valid
rt-behavior/trap/trap-function-inline-errors-rt
```

Also failing on that box, but **not** the same defect signature — several also
fail on x86_64/musl (2227), which rules out "aarch64+glibc only" for them. Triage
separately; do not fold them in:

```
rt-behavior/fs/bug159_listdir_notdir_error          also fails on x86_64/musl
rt-behavior/fs/func_fs_flush_valid                  also fails on x86_64/musl
rt-behavior/fs/func_fs_isBuffered_valid             also fails on x86_64/musl
rt-behavior/fs/func_fs_setBuffered_valid            also fails on x86_64/musl
rt-behavior/fs/fs-create-temp-file-rt               aarch64/glibc only
rt-behavior/fs/func_fs_createTempFile_valid         aarch64/glibc only
rt-behavior/project/project-fs-createTempFile-package-valid   aarch64/glibc only
```

Failing on **every** box tested. This section originally guessed these were
`scripts/linux-runtime-proof.sh` artifacts and said to confirm before believing
it. **Confirmed, and fixed** — see "The last 4 were harness bugs" above. The
guess named the right two mechanisms (no argv, and filesystem state a plain
`cd <root>` does not reproduce) but neither was verified at the time, and the
suspicion sat in this document as though it were a finding. It was not: it
became one only when each fixture was traced to its cause and the harness was
repaired. `rt-behavior/csv/csv-behavior` (reported "not found"),
`rt-behavior/fs/file-buffered-drain-integrity-rt`, and
`rt-behavior/fs/bug159_listdir_notdir_error` all wrote to a `target/` the box did
not have; `rt-behavior/project/project-entry-args-runtime` compared an `argv[0]`
the harness never reproduced. All four now pass on all three aarch64 boxes.

x86_64/musl additionally fails `json/json-behavior`,
`json/json-parse-deep-scalar-scan-rt`, `os/func_os_userName_valid` (Alpine has no
matching passwd entry for the test user, very likely environmental),
`fs/fs-listdir-order-rt`, `threads/thread-fs-listdir-order-rt` (directory
iteration order is not guaranteed and the goldens encode one order), and
`resources/resource-reclaim-loop-valid`. None were investigated.

## Not caused by bug-321

Established, not assumed:

- The executables are **byte-identical** before and after the bug-321 refactor.
  `shasum -a 256` over the pre- and post-refactor builds of
  `resource-state-valid` yields a single value, and the whole-corpus check
  (`scripts/linux-artifact-baseline.sh`, 11,154 hashes) reports no differences.
- The **pre-refactor** binary segfaults identically on the same box.
- `scripts/linux-runtime-proof.sh` run twice on 2223, once per compiler, produced
  **identical verdict lists** (446 pass / 21 fail both times).

## Why it was never caught

`scripts/artifact-gate.sh` and `scripts/test-accept.sh` run on the macOS host,
and the repo commits zero Linux artifact goldens. Nothing in the tree executed a
Linux binary on a Linux box until `scripts/linux-runtime-proof.sh` (added by
bug-321). This is exactly the coverage hole that bug's Validation Plan describes.

## Platform matrix — ISA, not libc

| box | arch | libc | result |
| --- | --- | --- | --- |
| 2222 Arch | aarch64 | glibc 2.35 | **SEGV** — 446 pass / 21 fail |
| 2223 Kali | aarch64 | glibc 2.42 | **SEGV** — 446 pass / 21 fail |
| 2224 Alpine | aarch64 | **musl** | **SEGV** — 446 pass / 21 fail |
| 2229 Alpine | riscv64 | musl | pass |
| 2227 Alpine | x86_64 | musl | pass |

The two aarch64/glibc boxes fail the **identical set of 21 fixtures** (`diff` of
the two failing-fixture lists is empty) despite seven years of glibc between
them. 2224 then reproduced it on aarch64/**musl** — and not just the one fixture: a
full runtime-proof pass there returns **446 pass / 21 fail with a failing set
that diffs empty against both glibc boxes**. Three aarch64 boxes, two different
libcs, byte-for-byte the same 21 failures; riscv64 and x86_64 pass on musl. That
removes libc from the picture entirely.

So the fault is in something aarch64-specific. All three Linux backends share the
same resource-drop lowering (`src/target/shared/`), so look at the aarch64
encoder / ABI realization of that path rather than at the shared code — and note
that `linux-aarch64` and `macos-aarch64` share `crate::arch::aarch64`, so macOS
may be affected too and simply is not covered by these fixtures' goldens.

## Suggested next steps

1. Check whether **macOS aarch64** reproduces — it shares `crate::arch::aarch64`
   with the failing target, and `scripts/test-accept.sh` passes there today, which
   would be informative either way (if macOS passes, the difference narrows to the
   Linux entry/teardown sequence on the same ISA).
2. Get a symbolized backtrace. 2222 has cores enabled and `coredumpctl`, but no
   `gdb`; installing gdb there is the cheapest path to a real frame list. The
   binaries carry no build-id, so symbolization needs the local `.ncode`/`.mir`
   dump to map the faulting return address.
3. Suspects, in order: the resource scope-drop / reclamation path (every confirmed
   fixture uses `RESOURCE`), then `_mfb_shutdown`, then arena unmap. A wild branch
   to `0x1000` is consistent with a resource drop-function pointer read from a
   record slot that was freed, never initialized, or offset wrongly. Memory notes
   `scope-drop-frees`, `trap-cleanup-double-free`, and `union-drop-codegen-nondeterminism`
   cover prior defects in exactly this area.
