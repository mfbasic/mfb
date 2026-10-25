# start

Start or attach to a program with mfb dbg, end the session, and drive it from a script

## Description

Give `mfb dbg` a program to start, or the process ID of one that is already
running, and it pauses that program and waits for commands at a `DBG>`
prompt:

```
mfb dbg <binary> [-- <args>...]
mfb dbg <pid>
```

- `mfb dbg <binary>` starts the program and pauses it before it runs a
  single instruction. Everything after `--` is passed to the program as its
  arguments.
- `mfb dbg <pid>` pauses the running program with that process ID, on
  every thread.

The program must be an MFBASIC executable built for the computer you are on.
`mfb dbg` refuses anything else, and never touches the program:

| You give it | It says |
| --- | --- |
| a program that is not MFBASIC, such as `/bin/ls` | `'/bin/ls' is not an MFBASIC executable` |
| an MFBASIC program built with `--target` for another platform | `'…' is built for ELF x86-64; this debugger runs Mach-O aarch64 programs` |
| a process ID that is not running | `no process with PID 999999` |

When a file exists with a name made only of digits, `mfb dbg 1234` starts that
file. Name it `./1234` either way to be clear.

`mfb dbg` runs on macOS, and on Linux on Arm (AArch64) or RISC-V (riscv64)
computers. On Linux on x86-64, and on Windows, it says `mfb dbg is not
supported` and stops; `mfb man debug-external` shows how to use gdb there.

On Linux, `mfb build` writes the example as `examples/debug/build/debug-glibc.out`
and `debug-musl.out`, one for each C library Linux systems use. Debug the one
your system runs — `debug-musl.out` on Alpine, `debug-glibc.out` on most
others — wherever these pages say `debug.out`.

A Linux app (`mfb build --app`) is an AppImage. Debug the program inside it:
`./<name>-glibc.AppImage --appimage-extract` writes it to
`squashfs-root/usr/bin/<name>`, and `mfb dbg` starts that. Or attach to the app
while it runs.

A program built without `--debug` can be debugged too, but `mfb dbg` says so
when it starts, because the names of your functions are missing:

```
note: this is not a debug build, so the program's function names are missing.
      Rebuild it with: mfb build --debug -O0 <project>
```

### Ending the session

`quit` ends the session, and so does the end of the input. How depends on how
the session began:

- A program `mfb dbg` started is stopped for good. If it never ran, it printed
  nothing and changed nothing.
- A program `mfb dbg` attached to carries on running from where it paused.

A command `mfb dbg` does not know is reported, and the prompt comes back.
`help` lists the commands. Every listing starts with a line naming its
columns.

### Driving it from a script

`mfb dbg` reads its commands from standard input, so a script, or another
program, can send them through a pipe. Two things make that easy:

- Every prompt is printed as soon as the debugger is ready, so a program
  reading the output can wait for `DBG> ` before sending the next command.
- When the commands come from a pipe, each one is printed after the prompt, so
  the output reads the same as a session typed by hand.

The examples in this topic all work that way, so you can paste them.

### Letting mfb dbg debug programs

macOS lets one program pause another only when the debugger is signed for it.
An `mfb` built from source is not, and `mfb dbg` stops with
`cannot attach to process …` and the command to fix it. From the root of the
MFBASIC repository, after every build of `mfb`:

```
sh scripts/sign-debugger.sh
```

Programs that `mfb build` makes need nothing extra.

Linux needs no signing. Many Linux systems, though, let a program pause only
the programs it started itself (their `kernel.yama.ptrace_scope` setting is 1
or more), so `mfb dbg <pid>` stops with `cannot attach to process …` and says
so. Start the program with `mfb dbg <binary>` instead, or run `mfb dbg` as the
system's administrator.

## Examples

Attach to the example while it runs, then let it go. `$!` is the process ID of
the program the shell just started in the background:

```sh macos
mfb build --debug -O0 examples/debug
examples/debug/build/debug.out 3000 20000 > rounds.txt 2> report.txt &
sleep 1
printf 'list thread\nq\n' | mfb dbg $!
kill $!
```

```
Attached to process 92393 (/Users/you/mfb/examples/debug/build/debug.out); paused.
DBG> list thread
ID | Name          | PC                 | Task
0  | isPrime+0x230 | 0x0000000100d14cd4 | MFB Main
DBG> q
Detached from process 92393; it continues running.
```

The program was in `isPrime` when it paused, and it keeps running afterwards.

## See also

- `mfb man debug`
- `mfb man debug breakpoints`
- `mfb man debug-external breakpoints` — stop in a function with lldb or gdb
- `mfb spec tooling debugger` — the exact behavior of `mfb dbg`
