# bug-360: on aarch64 + glibc 2.42, resource-using programs print correct output and then SIGSEGV at teardown

Last updated: 2026-07-19
Effort: medium
Severity: HIGH (every affected program exits 139 instead of 0, after doing its work correctly)
Class: Runtime / platform

Status: Open — found and characterized, not diagnosed
Regression Test: none yet; the eleven fixtures below already reproduce it — they
were simply never executed on this platform until now

Found while validating bug-321 (which is a pure reorganization and is **not** the
cause — see Not Caused By below). All ten `rt-behavior/resources/*` fixtures that
this box runs, plus `rt-behavior/trap/trap-function-inline-errors-rt`, run to
completion on `linux-aarch64` against **glibc 2.42**, print byte-correct output,
and then die with `SIGSEGV` (exit 139) during teardown. All eleven **pass** on
`linux-riscv64`/musl, from the same sources.

Because the program's own output is correct and complete, nothing upstream of
process exit is wrong; the fault is in the shutdown path (scope-drop / resource
reclamation / arena unmap / `_mfb_shutdown`).

## Reproduction

Box 2223 (Kali GNU/Linux Rolling, `ldd (Debian GLIBC 2.42-16) 2.42`, aarch64):

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

Confirmed failing on aarch64/glibc-2.42, all of which **pass** on
riscv64/musl — which is what localizes this to the aarch64+glibc pairing rather
than to the resource feature itself:

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

Note the CI/dev aarch64 Linux box has historically been 2222 (ArchLinux); 2223 is
Kali with glibc **2.42**, which is newer. Worth checking whether an older glibc
also reproduces — if not, this is a glibc-version regression and the version
boundary is the first thing to find.

## Suggested first steps

1. Re-run on an older-glibc aarch64 box (2222/2226) to establish whether this is
   glibc-version-dependent.
2. Get a backtrace. 2223 has no `gdb` and `ulimit -c` is 0; install one or enable
   cores.
3. The suspects, in order: the resource scope-drop / reclamation path (every
   confirmed fixture uses `RESOURCE`), then `_mfb_shutdown`, then arena unmap.
   Memory notes `scope-drop-frees` and `trap-cleanup-double-free` cover prior
   defects in exactly this area.
