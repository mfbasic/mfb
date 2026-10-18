# bug-1098: a Windows `--app` build always exits with status 0; `main`'s `Integer` result is dropped

Last updated: 2026-10-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (platform parity)

Status: Open
Regression Test: none yet

## Cause (coordinator, bug-staging-24)

- Rule: a program whose `main` returns `Integer` `n` exits with code `n` (`mfb spec language` error model, the entry table: "`FUNC ... AS Integer` succeeds with `n` | Exit code `n`"); the console build and the macOS `--app` build do.
- Where: `src/target/win_x86_64/app/mod.rs`, the Windows app bootstrap's `main_exit` label loads the immediate `0` and calls `ExitProcess`, so the worker's program-body result never reaches the process exit.
- Fix: the bootstrap exits with the status the program body produced, handed over the same way the macOS app bootstrap does it (find that path and mirror it); delete the hard-coded `0`.

Found by bs24-agent-b while proving bug-966 on box 206, reproduced by the bug-staging-24
coordinator, and placed in a later phase.

**The single correct behavior a fix produces:** a Windows `--app` build of a program whose
`main` returns `7` exits with status 7, as its console build and the macOS `--app` build do.

## Reproduction (main `7781cd71b`)

```
FUNC main() AS Integer
  RETURN 7
END FUNC
```

| build | where | exit status |
|---|---|---|
| `mfb build -q -target windows-x86_64` | box 206, PowerShell `Start-Process -Wait -PassThru` | 7 |
| `mfb build -q --app -target windows-x86_64` | box 206, same | **0** |
| `mfb build -q --app` (macOS) | `MFB_MACAPP_HEADLESS=1`, this machine | 7 |

## Where

`src/target/win_x86_64/app/mod.rs`, the bootstrap's `main_exit` label: it loads the
immediate `0` into the first argument and calls `ExitProcess`, so whatever the worker's
program body returned never reaches the process exit.
