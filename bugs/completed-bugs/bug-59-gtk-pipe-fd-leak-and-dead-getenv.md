# bug-59: Linux GTK app-mode `activate` leaks the pipe read-fd after `dup2`, and the backend declares a `getenv` import it never calls

Last updated: 2026-07-09
Effort: small (<1h)

Two LOW-severity defects in the Linux GTK app-mode backend, batched (same module).

**(1) Pipe read-fd leaked after `dup2(read, 0)`.** `emit_activate_handler` creates a
`pipe(fds)`, stashes the read fd in `ST_PIPE_READ_FD` and `dup2`s it onto fd 0, but never
`close`s the original read descriptor — indeed the GTK backend emits no `close` call
anywhere. So fd 0 and `ST_PIPE_READ_FD` are two distinct open ends of the same pipe for
the life of the process: one fd leaked, and stdin EOF/hangup semantics are muddied
(closing one end does not signal EOF while the other stays open). `activate` runs once
per launch, so it is a single leaked fd, not unbounded.

**(2) Dead `getenv` import.** `getenv` is listed in `lib_for`'s LIBC arm and declared in
`app_mode_imports`, but no emitter calls `call_external("getenv")` in the GTK backend —
only `setenv` is used (to disable GTK a11y / IM modules). The import is a leftover from an
earlier design that read env; it adds an unused dynamic-symbol dependency.

The single correct behavior a fix produces: the redundant read fd is closed after `dup2`
(or `ST_PIPE_READ_FD` simply *is* fd 0), and the plan declares only imports the backend
actually references.

References:

- `src/target/linux_gtk/bootstrap.rs:emit_activate_handler` (`:205-214`): `pipe`,
  `dup2(read, 0)`, no `close(read)`. `grep '"close"' src/target/linux_gtk/` → no match.
- `src/target/linux_gtk/mod.rs:205` (`lib_for` LIBC arm lists `getenv`) and `:674`
  (`app_mode_imports` declares `(LIBC, "getenv")`). No `call_external("getenv")` in the
  backend; `setenv` at `bootstrap.rs:54-61` is the only env call.
- Found during the goal-01 compiler source review of `src/target/linux_gtk/`.

## Failing Reproduction

- (1) Build any GTK app-mode program and inspect `/proc/<pid>/fd`: the pipe appears twice
  (fd 0 and the original read fd), and closing the write end does not yield EOF on stdin
  readers because the extra read end holds the pipe open.
- (2) Dump the app-mode import plan (`mfb ... --nplan` / inspect the dynamic symbols):
  `getenv` appears though nothing references it.

- Observed: one leaked fd + muddied stdin EOF (1); an unused `getenv` dynamic symbol (2).
- Expected: exactly one read end (fd 0) with correct EOF semantics; no unreferenced import.

Contrast: the write fd is intentionally retained (the key handler writes committed input
to it) — keeping it is correct; only the redundant read fd is stray. `setenv` in the same
`lib_for` arm is genuinely called; every other listed symbol has a call site.

## Root Cause

(1) `emit_activate_handler` omits `close(read_fd)` after `dup2`. (2) `getenv` survived a
refactor that removed the env-read path; only `setenv` remained, but the import list and
`lib_for` arm were not pruned.

## Goal

- After `dup2(read, 0)`, the original read fd is closed (or the design uses fd 0 directly
  as `ST_PIPE_READ_FD`), so exactly one read end exists and stdin EOF works.
- `app_mode_imports` / `lib_for` declare no `getenv` (unless a caller is added).

### Non-goals (must NOT change)

- The write fd retention and the key-handler pipe writes.
- The `setenv` a11y/IM-disable calls.

## Blast Radius

- `emit_activate_handler` (`bootstrap.rs`) — add the `close`, and a `close` import if the
  backend gains its first close call.
- `lib_for` arm + `app_mode_imports` (`mod.rs`) — drop `getenv`.
- No runtime behavior change beyond the fd fix; the import prune is metadata-only.

## Fix Design

(1) After `dup2(read_fd, 0)`, emit `close(read_fd)` (adding a `close` LIBC import), or
store fd 0 as `ST_PIPE_READ_FD` and skip the extra descriptor entirely. (2) Remove
`getenv` from `lib_for`'s LIBC arm and from `app_mode_imports`.

## Phases

### Phase 1 — audit + test

- [x] Confirm the double read-fd in `/proc/<pid>/fd` and the unreferenced `getenv` symbol.

### Phase 2 — the fix

- [x] Close the redundant read fd (or reuse fd 0); drop the `getenv` import.

### Phase 3 — validation

- [x] Regenerate GTK goldens (delta = close instruction + import list); `scripts/test-accept.sh`;
      verify a single read end and working stdin EOF in the built app.
      (No committed Linux-GTK goldens exist — the delta is exercised by unit tests instead;
      runtime `/proc/<pid>/fd` verification is not possible on this macOS host.)

## Validation Plan

- Regression test(s): an fd-count / EOF check on the built GTK app (or a codegen assertion
  the activate handler closes the read fd); an import-list golden without `getenv`.
- Runtime proof: `/proc/<pid>/fd` shows one read end; stdin EOF fires on write-end close.
- Doc sync: none expected.
- Full suite: `scripts/test-accept.sh`.

## Summary

The GTK `activate` handler leaves the pre-`dup2` pipe read fd open (one leaked fd + fuzzy
stdin EOF), and the backend carries a `getenv` import it never calls. Both fixes are
local: close the redundant fd (or use fd 0 directly) and prune the dead import.

## Resolution

Fixed in `src/target/linux_gtk/`.

**(1) Pipe read-fd leak.** `emit_activate_handler` (`bootstrap.rs`) now, after
`dup2(read, 0)`, reloads the read fd from the pipe-fds stack slot (`ldr_u32 x0, [sp, #16]`)
and calls `close` on it, then records the surviving read end (fd 0) in `ST_PIPE_READ_FD`.
The read fd is loaded fresh from the stack for both `dup2` and `close` — never held in a
caller-saved register across either `bl` (Native Codegen Register Lifetimes). Correctness
of the descriptor closed: `pipe(2)` never returns fd 0 here (fds 0/1/2 are open at process
start), so the original `read` descriptor is distinct from the fd-0 copy `dup2` creates;
closing the original leaves exactly one read end, so closing the write end now signals
stdin EOF/hangup. The write-fd retention and the key-handler pipe writes are unchanged, as
are the `setenv` a11y/IM-disable calls (non-goals).

**(2) Dead `getenv` import.** Removed `getenv` from `lib_for`'s LIBC arm and from
`app_mode_imports` (`mod.rs`); added `close` to both (the backend's first `close` caller).

**Tests.** `bootstrap.rs::activate_closes_redundant_pipe_read_fd_after_dup2` asserts the
emitted sequence: exactly one `bl dup2` and one `bl close`, close after dup2, and both
taking the read fd from stack offset 16 (the read end, not the write end at offset 20).
`mod.rs::import_tests` asserts `getenv` is gone, `close` is present, and `lib_for("close")`
maps to libc. Both new tests fail before the fix (no `close` call / `getenv` present) and
pass after.

**Commands.** `cargo test -p mfb linux_gtk` → 5 passed (incl. the 2 new).
`cargo test -p mfb --test linux_app_mode` → 4 passed.

**Goldens.** No shift: there are no committed Linux-GTK app-mode goldens (only macOS app
goldens, a separate `macos_aarch64` backend, untouched). The `tests/linux_app_mode.rs`
integration test regenerates its `.nplan` at runtime and does not assert on
`getenv`/`close`, so it is unaffected.

**Runtime note.** This host is macOS aarch64 and cannot execute the Linux GTK ELF, so the
fix is proven by the emitted-instruction assertions and the descriptor-uniqueness reasoning
above; `/proc/<pid>/fd` verification was not possible here.
