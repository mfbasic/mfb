# bug-159 — `emit_errno_error_mapping` has no terminating branch → unmapped errnos misclassified as ErrInvalidArgument across 6 fs functions

Last updated: 2026-07-12
Severity: MEDIUM — wrong error code (and wrong message) from six fs builtins on any errno outside ENOENT/EACCES/EEXIST.
Class: Correctness.
Status: Open

## Finding

`src/target/shared/code/fs_helpers.rs:43-48` (`emit_errno_error_mapping`, the
catch-all `err_output` block). Unlike the sibling `emit_fs_path_errno_error_mapping`
(which ends with `abi::branch(done)`, `fs_helpers.rs:151`), the catch-all sets
`ERR_OUTPUT_CODE` + message but emits **no trailing branch**. `push_error_message_address`
also emits no branch (`data_objects.rs:40-66`). Every one of the six callers
places `abi::label(&invalid)` immediately after the call with no intervening
branch, so control falls straight through `err_output` into the `invalid` block,
overwriting `RESULT_VALUE_REGISTER` with `ERR_INVALID_ARGUMENT_CODE` + message.
The `err_output` stores are dead; the returned `Result` is `ErrInvalidArgument`.

Callers affected: `fs_helpers_atomic.rs:216, 924, 1423` and
`fs_helpers_paths.rs:1271, 1476, 1742` — i.e. `fs::createTempFile`,
`fs::writeText(path)`/`writeBytes(path)`, `fs::listDirectory`,
`fs::canonicalPath`, `fs::isWithin`. (The atomic-write caller,
`fs_helpers_atomic.rs:661`, is the only site that compensates with an explicit
`abi::branch(&done)` at :666 — confirming the intended contract that the callee
should branch.)

## Trigger

Any of those six functions failing with an errno other than ENOENT(2)/EACCES(13)/
EEXIST(17): disk full (ENOSPC 28), too many fds (EMFILE 24), not-a-directory
(ENOTDIR 20 — e.g. `fs::listDirectory("/etc/hosts")`), I/O error (EIO 5), ELOOP.
The user sees `ErrInvalidArgument` with the invalid-argument message instead of
the correct `ErrOutput`/errno-appropriate error. Contrast: the `fs_path` errno
variant maps the same errnos to `ErrOutput` correctly.

## Fix

Append `instructions.push(abi::branch(done));` at the end of
`emit_errno_error_mapping` (mirror the fs-path variant), or add the branch at the
six fall-through call sites. Add a runtime test that `fs::listDirectory` on a
regular file (ENOTDIR) returns the non-InvalidArgument error.
