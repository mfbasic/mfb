# bug-500: `process::spawn` env-replace clear loop never terminates on an `environ` entry with no `=` (or leading `=`) → hang + unbounded child allocation

Last updated: 2026-09-03 (fixed)
Effort: small (<1h)
Severity: HIGH
Class: security (denial of service — hang + memory exhaustion)

Status: FIXED (d6817d397) — found in audit-3, Surface 4 OS-02; agent-demonstrated 17 GB/19 s, mechanism code-verified by the lead

Regression Test: `tests/rt_process_spawn_env_replace_bogus_environ.rs` — execs the built binary through a C launcher with a raw envp (`"BOGUS_NO_EQUALS"` and `"=C:=C:\\"`) and calls `process::spawn(..., envReplace=TRUE)`, asserting a bounded exit and that the entry AFTER the bogus one was still cleared.

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

## STATUS: FIXED (d6817d397)

Reproduced exactly as documented before fixing: with the unfixed compiler, a C
launcher exec'ing the program with `envp = {"BOGUS_NO_EQUALS", "PATH=…", NULL}`
left the parent in `S` (blocked in the self-pipe `read()`) and the fork child in
`R` at 2.5 GB RSS after 4 s; the `"=C:=C:\\"` shape reached 12 GB in 4 s. A
well-formed control (`"HOME=/tmp"`) finished in <300 ms with `child=0`.

Fix (`emit_child_apply_env`, `src/codegen/builtins/process/gen_unix.rs`): the
clear walks `environ` by index. An entry whose scan ended at NUL (no `=`) or
whose name is empty (leading `=`) is stepped over — there is nothing `unsetenv`
can remove — and after `unsetenv` the index stays put only when `environ[idx]`
actually changed. Every iteration either removes an entry or advances, so the
loop, and with it the never-freed name-buffer allocation, is bounded by the
length of `environ`. The "never free in the fork child" strategy is kept for the
now-bounded case, per Non-goals.

Evidence: RED test (timeout, both shapes) → GREEN in 2.2 s; the repro finishes
in <300 ms with `child=0` for both shapes and the control; `cargo test --bin mfb`
3774 passed / 0 failed; `artifact-gate.sh … process` 7 goldens, 0 diffs after
regenerating the four Unix `.ncodesum` (windows-x86_64 byte-identical — the
emitter is Unix-only); `test-accept.sh` on all 22 `process` fixtures (incl.
`spawnenv`, the well-formed `envReplace=TRUE` case) passed; `cargo check
--all-targets` clean.

Deviation from the doc's "Best fix": kept per-entry `unsetenv` (index-advancing)
rather than `clearenv()` + re-add — `clearenv` does not exist on macOS, and the
bounded per-entry loop preserves the existing portable strategy.
