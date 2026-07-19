# bug-360: on aarch64, resource-using programs print correct output and then SIGSEGV at teardown

Last updated: 2026-07-19
Effort: medium
Severity: HIGH (every affected program exits 139 instead of 0, after doing its work correctly)
Class: Runtime / platform

Status: Open — found and characterized, not diagnosed
Regression Test: none yet; the eleven fixtures below already reproduce it — they
were simply never executed on this platform until now

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

Failing on **every** box tested, and most likely artifacts of
`scripts/linux-runtime-proof.sh` rather than product defects — it does not supply
argv, and some fixtures depend on filesystem state a plain `cd <root>` does not
reproduce. Confirm the harness before treating any of these as real:
`rt-behavior/csv/csv-behavior` (reports "not found"),
`rt-behavior/fs/file-buffered-drain-integrity-rt`,
`rt-behavior/project/project-entry-args-runtime`.

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
