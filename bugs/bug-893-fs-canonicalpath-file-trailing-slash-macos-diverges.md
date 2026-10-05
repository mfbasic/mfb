# bug-893: fs::canonicalPath / fs::isWithin accept "file/" and "file/.." on macOS but raise ErrInvalidPath on Linux

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: the Linux half was measured by the review agent in podman containers only; the coordinator did not re-run it.
Regression Test: tests/rt-behavior/fs/<new> (canonicalPath of `<regular file>/`, `/.`, `/..`)

On macOS, `fs::canonicalPath` resolves a path that uses a regular file as a directory (`d/file/`,
`d/file/.`, `d/file/..`) to `d/file` or `d`. On Linux (glibc and musl) the same calls raise
`ErrInvalidPath`. `fs::isWithin`, which canonicalizes both arguments, splits the same way. The host
itself refuses such a path everywhere: `fs::openFile("d/file/")` raises `ErrInvalidPath` on macOS too.
So on macOS `canonicalPath` hands back a path for an input that no `fs` member can open.

The spec promises the opposite:

> Every `fs` member that hands a path to the host reports a refusal through one mapping, so the same
> condition raises the same `errorCode` whichever member met it and whichever host it ran on. There are
> no per-platform differences in the codes an `fs` member raises.

The man page lists "a file used as a directory" as `ErrInvalidPath` for `canonicalPath`.

The correct behavior: `canonicalPath`/`isWithin` raise `ErrInvalidPath` on macOS for a path where a
non-final component (or a final component with a trailing `/`, `/.` or `/..`) names a non-directory,
the same as on Linux and the same as the opens on macOS. macOS libc `realpath(3)` resolves these
lexically (checked with a C probe, below), so the fix has to add its own check rather than trust
`realpath`.

References:

- `src/docs/spec/stdlib/21_fs-host-errors.md` (intro and the "a file used as a directory → ENOTDIR →
  ErrInvalidPath" row).
- `./mfb man fs canonicalPath`: "ErrInvalidPath if path cannot name an entry — a file used as a
  directory …".
- Found during spec-hardening batch B08.

## Failing Reproduction

`/tmp/hardening/B08/repro-canon/src/main.mfb` creates `<tempdir>/mfb-b08-canon/file` and then calls
`canonicalPath` on `file/`, `file/.`, `file/..`, `isWithin(d, d/file/)`,
`isWithin(d/file/, d/file/x)` and `fs::openFile(d/file/)`. Each call is in its own TRAP helper.

```
cd /tmp/hardening/B08/repro-canon && /tmp/hardening/mfb build && ./build/repro-canon.out
/tmp/hardening/mfb build --target linux-x86_64
mkdir -p ~/hardening-b08-tmp && cp build/repro-canon-glibc.out build/repro-canon-musl.out ~/hardening-b08-tmp/
podman run --rm -v ~/hardening-b08-tmp:/w debian:bookworm-slim /w/repro-canon-glibc.out
podman run --rm -v ~/hardening-b08-tmp:/w alpine:3 /w/repro-canon-musl.out
```

- Observed (macos-aarch64; `$TMP` is the canonical temp dir):
  ```
  $TMP/mfb-b08-canon/file/ -> $TMP/mfb-b08-canon/file
  $TMP/mfb-b08-canon/file/. -> $TMP/mfb-b08-canon/file
  $TMP/mfb-b08-canon/file/.. -> $TMP/mfb-b08-canon
  isWithin($TMP/mfb-b08-canon, $TMP/mfb-b08-canon/file/) -> TRUE
  isWithin($TMP/mfb-b08-canon/file/, $TMP/mfb-b08-canon/file/x) -> ERR 77030002
  open file/ -> ERR 77030002
  ```
- Observed (linux-x86_64 glibc and musl, podman amd64 containers): every line `-> ERR 77030002`.
- Expected on every host: every line `-> ERR 77030002` (`ErrInvalidPath`).

| Environment | libc | Result |
| --- | --- | --- |
| macos-aarch64 (Darwin 24.6) | libSystem | returns a path ✗ |
| linux-x86_64 (debian bookworm) | glibc | ErrInvalidPath ✓ |
| linux-x86_64 (alpine 3) | musl | ErrInvalidPath ✓ |
| windows | — | not run |

Host probe (`/tmp/hardening/B08/rp.c`, plain `realpath(3)`) on macOS prints
`.../file/ -> .../file`, `.../file/. -> .../file`, `.../file/.. -> .../t`. Python's
`os.path.realpath(..., strict=True)` on the same Mac gives `[Errno 20] Not a directory`, because
Python does its own resolution.

## Root Cause

`src/codegen/builtins/fs/gen_canonical.rs` calls the host `realpath` and maps a failure through
`emit_fs_host_error_mapping` (`FsHostOp::Resolve`). On macOS, `realpath` does not fail for
`<file>/`, `<file>/.` or `<file>/..`, so there is no errno to map and the success path returns the
lexically collapsed result. The mapping is correct. The divergence comes in before it, from the
macOS `realpath` contract. `fs::isWithin` (`func_is_within.rs`) and `fs::openWithin`'s root
canonicalization go through the same `realpath`. Not exercised for `openWithin`.

## Goal

- On macOS, `canonicalPath`/`isWithin` raise `ErrInvalidPath` for the three repro shapes. Linux
  output is unchanged.

### Non-goals (must NOT change)

- A trailing `/` on a real directory (`d/dir/`) still canonicalizes to `d/dir` on every host.
- Symlink resolution and the rest of the error mapping.

## Blast Radius

- `fs::canonicalPath`, `fs::isWithin`: fixed by this bug.
- `fs::openWithin` (root canonicalized with `realpath`): latent, same hazard, not exercised.
- Every open/read/write member: unaffected (the host open itself returns ENOTDIR; verified in the
  `/tmp/hardening/B08/fsx` matrix, `dirslash` rows).
