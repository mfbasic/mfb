# bug-29: `mfb init` `write_new_file` has a check-then-write TOCTOU and follows symlinks

Last updated: 2026-07-08
Effort: small (<1h)

`src/cli/init.rs::write_new_file` (`:44-50`) guards against overwrite with
`if path.exists() { return Err(...) }` then `fs::write(path, contents)`. Between the
`exists()` check and the write, a file or symlink can appear at `path`; `fs::write`
then **follows a symlink** and overwrites its target. `exists()` also follows
symlinks, so a pre-existing symlink to a nonexistent target passes the check and the
write lands on the (attacker-chosen) link target.

The single correct behavior a fix produces: file creation is atomic and refuses to
overwrite (or follow a symlink to) an existing path, closing the check-write window.

Severity LOW: reachable only via `mfb init` in a directory an attacker can race, and
the content written is fixed hello-world/manifest template text (not
attacker-controlled) — so impact is limited to clobbering a victim-owned file the
attacker can point a symlink at. Filed because it is the same non-atomic
symlink-following-write class as bug-27 and the fix is trivial.

References:

- `src/cli/init.rs:44-50` (`write_new_file`: `exists()` check then `fs::write`).
- Related pattern (package install, higher severity): bug-27.
- Found during goal-01 review of `src/cli/**`.

## Failing Reproduction

In a directory an attacker can write to, pre-plant `hello.mfb` (the init target) as
a symlink to a victim file, then the victim runs `mfb init`:

- Observed: `path.exists()` is true only if the link resolves; a dangling symlink
  passes the check and `fs::write` follows it, clobbering the target with template
  text. Even a racing create between check and write is followed.
- Expected: `create_new` fails with `AlreadyExists` → the refusal message; no
  symlink is followed.

## Root Cause

Non-atomic create: `exists()` + `fs::write` is a TOCTOU, and neither refuses to
follow a symlink. There is no `O_EXCL`/`create_new`.

## Goal

- `write_new_file` creates the file atomically, refusing (with the existing message)
  if anything is already at `path`, and never follows a symlink.

### Non-goals (must NOT change)

- The refusal message / behavior for the common "file already exists" case.

## Blast Radius

- `write_new_file` and its callers in `init.rs` (manifest + hello-world writes).

## Fix Design

Use `OpenOptions::new().write(true).create_new(true).open(path)` and map an
`AlreadyExists` error to the existing "refusing to overwrite" message; write
`contents` to the returned handle. `create_new` is `O_EXCL` and does not follow a
final-component symlink.

## Phases

### Phase 1 — failing test + audit

- [ ] Test: a pre-existing symlink/file at the target causes a clean refusal and no
      target clobber. Confirm the current code follows the symlink.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Switch `write_new_file` to `create_new`.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; `mfb init` in a clean dir still succeeds; in a dir
      with the target present it refuses.

## Validation Plan

- Regression test(s): the symlink/race refusal test.
- Runtime proof: `mfb init` twice in a dir — second run refuses without following.
- Full suite: `scripts/test-accept.sh`.

## Summary

A trivial TOCTOU/symlink-follow in `mfb init`'s file writer; `create_new` fixes it
and preserves the refusal behavior.
