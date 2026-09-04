# bug-500: `process::spawn` env-replace clear loop never terminates on an `environ` entry with no `=` (or leading `=`) → hang + unbounded child allocation

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (denial of service — hang + memory exhaustion)

Status: Open (found in audit-3, Surface 4 OS-02; agent-demonstrated 17 GB/19 s, mechanism code-verified by the lead)

Regression Test: an rt fixture that execs the built binary with a `"BOGUS_NO_EQUALS"` envp entry and calls `process::spawn(..., envReplace=TRUE)`, asserting a bounded exit.

## Summary

The fork child's environment-clear loop for `process::spawn(args, cwd, env,
TRUE)` removes `environ[0]` by deriving its name (scan to `=` or NUL) and calling
`unsetenv(name)`, then restarts from `environ[0]`. The termination argument
("`unsetenv` shrinks the array") fails for two entry shapes a caller-supplied
`envp` can contain — an entry with no `=`, and an entry beginning with `=` — so
`environ[0]` never changes and the loop spins forever. Each iteration also
arena-allocates a name buffer that is deliberately never freed, so the child's RSS
grows without bound while the parent blocks in `read()` on the exec self-pipe.

## Mechanism

```rust
// src/codegen/builtins/process/gen_unix.rs:315-325
platform.emit_external_call("unsetenv", ...)?;      // unsetenv(name)
// environ shifted down in place; environ[0] is the next entry. Reload ep ...
platform.emit_environ_pointer(...)?;
instructions.extend([ abi::move_register(&ep, abi::return_register()),
                      abi::branch(&clear_loop) ]);   // restart from environ[0]
```

- **No `=`** (`"BOGUS_NO_EQUALS"`): the name scan stops at NUL, so
  `unsetenv("BOGUS_NO_EQUALS")` matches nothing → `environ[0]` unchanged → spin.
- **Leading `=`** (`"=C:=C:\\"`): `nlen == 0`, and both glibc and Darwin
  `unsetenv("")` fail `EINVAL` and remove nothing → spin.

The per-iteration buffer is documented as never freed (`gen_unix.rs:180-184`,
"Runs in the fork child … never freed"), so the spin is also a memory bomb. The
kernel does not validate `envp` strings, so a CGI/inetd/systemd/CI parent can
hand the MFBASIC process such an entry.

## Reproduction

Agent-demonstrated: a C launcher execs the MFBASIC binary with
`envp = {"BOGUS_NO_EQUALS", "PATH=/usr/bin:/bin", NULL}`; the program calls the
4-arg `process::spawn(..., TRUE)` → child spins to ~17 GB RSS in ~19 s, parent
wedged. Lead code-verified the reload-and-restart loop and the never-freed
allocation.

## Best fix

Advance the loop by index, not by re-reading `environ[0]`: iterate `environ[i]`
and only restart/decrement when `unsetenv` actually removed an entry; skip an
entry with no `=` or with `nlen == 0` (there is nothing to unset). Equivalently,
build the child env by `clearenv()` + re-adding the requested vars rather than
unsetting one at a time. Bound or reuse the name buffer regardless.

## Non-goals

No change to `process::spawn` semantics for a well-formed environment; keep the
"never free in the fork child" strategy for the *bounded* case (it is correct once
the loop terminates).

## Prior art

None (searched `unsetenv`, `envReplace`, `clear loop`, `environ` across `bugs/`,
`bugs/completed/`, `audit-1-*`, `audit-2-*`).
