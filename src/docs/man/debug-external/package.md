# debug-external

Debugging and profiling an MFBASIC program with lldb, gdb and the platform tools

## Synopsis

```
mfb build --debug [-O0] [path]
mfb man debug-external [topic]
```

## Imports

`debug-external` is a documentation topic, not an importable package. No
`IMPORT` is needed: everything here happens at build time and in the debugging
tools.

## Description

MFBASIC comes with its own debugger, `mfb dbg`, and that is the place to start:
`mfb man debug` explains it. This topic is for the times you want a tool from
outside: lldb or gdb, a profiler, or a tool that turns a crash address into a
function name, on macOS, Linux or Windows.

`mfb build --debug` makes those tools useful: the executable carries the **name
of every function** in your program, so debuggers, profilers and address-lookup
tools show `isPrime` and `runRound` instead of bare addresses. The same build
prints a debug report when the program exits (`mfb man debug report`).

That gets you breakpoints by function name, backtraces, profiles and crash
address lookup on every platform. It does **not** give you source lines or
variables: these tools cannot step one source line at a time, show
`main.mfb:42`, or print the value of a variable. They work at the level of
functions.

`mfb test --debug` builds its test programs the same way.

### The example program

Every page in this topic debugs the same program as `mfb man debug`,
`examples/debug` in the MFBASIC repository. It counts the primes in a run of
ranges and prints one line per round:

```
main -> runRounds -> runRound -> countPrimes -> isPrime
                              -> next_seed
                  -> formatLine
                  -> summarize
                  -> roundWorker -> runRound        (threads mode)
```

It takes up to three optional arguments: `rounds` (default 5), `size` (default
20000), and `threads`. With `threads` set to `1`, each round runs on its own
worker thread (`roundWorker`). Large values keep it running long enough to
attach a tool to it: `debug 3000 20000` runs for about ten seconds.

Every script in this topic runs from the root of the repository and builds the
example itself. Each one was run, as written, on macOS, on Linux (Debian 12,
aarch64) and on Windows 11 (x86-64).

### Build for debugging with -O0

Build with `--debug -O0` while debugging, then list the names the build
carries. macOS (the executable is `build/debug.out`):

```sh macos
mfb build --debug -O0 examples/debug
nm -n examples/debug/build/debug.out | grep ' _mfb_fn_'
```

Linux (one executable per C library, `build/debug-glibc.out` and
`build/debug-musl.out`; run the one that matches your system):

```sh linux
mfb build --debug -O0 examples/debug
nm -n examples/debug/build/debug-glibc.out | grep ' _mfb_fn_'
```

Windows, in PowerShell (the executable is `build\debug.exe`):

```powershell windows
mfb build --debug -O0 examples\debug
nm -n examples\debug\build\debug.exe | Select-String ' _mfb_fn_'
```

Each lists the ten functions of the example:

```
00000001000008d0 t _mfb_fn_next_5Fseed
0000000100000aa4 t _mfb_fn_isPrime
0000000100000d80 t _mfb_fn_countPrimes
0000000100001108 t _mfb_fn_runRound
00000001000012c4 t _mfb_fn_roundWorker
0000000100001374 t _mfb_fn_formatLine
0000000100001824 t _mfb_fn_logPath
0000000100001ee0 t _mfb_fn_summarize
0000000100002278 t _mfb_fn_runRounds
00000001000084e0 t _mfb_fn_main
```

The addresses differ by platform. On Windows the letter is `T`, not `t`.

The default optimization level merges small functions into their callers.
`next_seed` is one of them: in a default build no call to it remains, so a
breakpoint on it never stops and it never appears in a backtrace. `-O0` keeps
every function a separate call. Profile the default build instead (see
`mfb man debug-external profiler`), because that is the code your program runs.

### How your functions are named

A debugger shows a function under the name it has in the executable:

| You wrote | The tools show | On macOS (lldb, atos, sample) |
| --- | --- | --- |
| `FUNC isPrime` | `_mfb_fn_isPrime` | `mfb_fn_isPrime` |
| `FUNC next_seed` | `_mfb_fn_next_5Fseed` | `mfb_fn_next_5Fseed` |
| the program's entry | `_main` | `main` |

- Every function you write is `_mfb_fn_` followed by its name.
- Letters and digits are kept. Every other character, including `_`, becomes
  `_` plus its two-digit hexadecimal code, so `_` is `_5F`.
- The macOS tools drop the leading `_`. gdb on Linux and Windows keeps it.
- Names that start `_mfb_rt_`, `_mfb_arena_` or `_mfb_` without `fn_` belong to
  the MFBASIC runtime: printing, string building, memory, the debug report.
- A function can appear a second time with a `$own` suffix, for example
  `_mfb_fn_directionFor$own4`. That is a second version of the same function
  that the compiler made for some of its callers. To stop in every version,
  use a pattern breakpoint (`mfb man debug-external breakpoints`).

### The tools

| Platform | Debugger | Profiler | Address lookup | Install |
| --- | --- | --- | --- | --- |
| macOS | lldb | `sample` | `atos`, `nm` | Xcode Command Line Tools: `xcode-select --install` |
| Linux | gdb | gdb, sampling | `addr2line`, `nm`, gdb | Debian/Ubuntu: `sudo apt install gdb binutils` |
| Windows | gdb | gdb, sampling | gdb, `nm` | w64devkit (below) |

On Windows, Visual Studio and WinDbg read names only from a PDB file, which
MFBASIC does not write, so they show bare addresses. Use gdb from w64devkit, a
free toolkit that needs no installer and no administrator rights. This
PowerShell script downloads it into your home directory and puts it on the
`PATH` for the current session:

```powershell windows-setup
curl.exe -sSL -o "$HOME\w64devkit.exe" https://github.com/skeeto/w64devkit/releases/download/v2.10.0/w64devkit-x64-2.10.0.7z.exe
Start-Process "$HOME\w64devkit.exe" -ArgumentList '-y', "-o$HOME" -Wait -NoNewWindow
$env:PATH = "$HOME\w64devkit\bin;$env:PATH"
gdb --version | Select-Object -First 1
```

Add `%USERPROFILE%\w64devkit\bin` to your user `PATH` to keep it. A newer
release from https://github.com/skeeto/w64devkit/releases works the same way.

On Linux, gdb attaches to a running program only if the system allows it. If
gdb reports `ptrace: Operation not permitted`, the `kernel.yama.ptrace_scope`
setting forbids attaching. Start the program under gdb instead, or ask an
administrator to change the setting.

## Topics

- `breakpoints` — stop the program when it reaches a function.
- `backtraces` — see the chain of calls that led to where the program is.
- `profiler` — find the functions the program spends its time in.
- `lookup` — turn an address from a crash or a log into a function name, and back.

## Errors

No errors. This topic describes tools, not MFBASIC functions.

## See also

- `mfb man debug` — the MFBASIC debugger, `mfb dbg`
- `mfb man debug-external breakpoints`
- `mfb man debug-external backtraces`
- `mfb man debug-external profiler`
- `mfb man debug-external lookup`
- `mfb man debug report`
- `mfb man optimizations`
