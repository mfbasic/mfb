# bug-92 — Source scan swallows genuine EACCES: build fails with no diagnostic

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G1).
**Severity:** LOW — confusing silent failure, not wrong output.
**Class:** correctness / silent failure.

## Finding

`src/ast/manifest.rs:318-329` — `collect_selected_source_files` uses
`ErrorKind::PermissionDenied` as an in-band sentinel meaning "diagnostic
already shown for an outside-project path" (the sentinel is constructed with
`io::Error::new(PermissionDenied, …)` *after* `show_diagnostic` in
`collect_mfb_files`). The caller therefore suppresses reporting for that error
kind.

But a **real** EACCES from `fs::read_dir` / `fs::canonicalize`
(manifest.rs:412, 415, 384) has the same `ErrorKind`, was never reported, and
takes the same suppressed path: the build returns Err and exits 1 printing
nothing at all.

## Trigger

```
chmod 000 src/subdir   # in a project whose sources glob includes it
mfb build              # exits 1, zero output
```

## Fix sketch

Replace the in-band sentinel with a dedicated error type (or a bool flag on a
custom error), so genuine permission errors surface as diagnostics like every
other I/O failure.
