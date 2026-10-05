# inspecting

List the program's functions, threads and registers, and read its memory

## Description

```
list func [<filter>...]   l func …
list thread               l thread
list reg …                l reg …
show mem <where> [<size>]
show error [<thread>]
```

### Listing functions

`list func` prints one function per line, under the header
`Type Address Symbol Written name`, and after the list a legend for the types
it shows:

| Type | The function is |
| --- | --- |
| `MU` | MFB User: one you wrote, in your program or a package it imports |
| `MB` | MFB Builtin: part of a built-in package, such as `datetime` |
| `MR` | MFB Runtime: part of the MFBASIC runtime, which every program carries |
| `SL` | System Linked: a system library function the program imports, such as `malloc` |
| `SD` | System Dynamic: in a library the program loaded while it ran |

`Symbol` is the function's name in the executable, the one lldb and gdb use
(`mfb man debug-external` explains how your names are spelled there).
`Written name` is the name as you wrote it, so `_mfb_fn_next_5Fseed` shows as
`next_seed`, and a second version of a function (a name ending in `$own` and a
number) shows under the function's own name. A runtime function that does the
work of a built-in package function shows that function's name, so
`_mfb_rt_io_io_print` shows as `io::print`; the rest of the runtime shows `-`.
A built-in package's function shows as the package and the function:
`color::brighten` for one you can call, and `datetime::#normInstant`, with a
`#`, for one the package keeps to itself. You cannot call a `#` function, but
`breakpoint datetime::#normInstant` stops in it.

When one of your functions and a native one have the same name, the written
name says which is which: `main <mfb>` is your `main`, and `main <native>` the
program's native entry, which calls it. Breakpoints, stops, `dis` and `bt` tag
the name the same way. The tag is not part of the name: `breakpoint main` stops
in both.

`Address` is where the function is in the running program. A program `mfb dbg`
started is paused before it has loaded its system libraries, so every `SL`
function shows `-` there; attach to a running program to see where they are.
When a program calls a system function from more than one place in the runtime,
that function is listed once for each.

`list func` lists every function; name a type or a function after it to see
fewer. `list func MU` lists only the functions you wrote, `list func fib` the
function named `fib`, and `list func *norm*` every function with `norm` in its
name, `*` standing for any characters and `?` for any one. A name is matched
against both the `Symbol` and the `Written name`. Give several to see more:
`list func MU MB` lists your functions and the built-in packages'.

`SD` functions appear once the program has loaded their library, so a program
just started has none. Most system frameworks, such as `CoreFoundation` and
`Network`, are loaded with every program, so a library the program asks for
later is listed as `SD` only when nothing loaded with the program already
needed it.

Your functions are named only in a program built with `--debug`; in any other,
`list func` shows only the `SL` functions.

### Listing threads

`list thread` prints one thread per line, under the header `ID Name PC Task`.
`ID` is the number the other commands use for the thread. `Name` is the
name the thread gave itself (system threads often have one); otherwise the
function it is in, such as `isPrime+0x278`; otherwise `thread-` and the
system's number for it. `PC` is the address of the next instruction it will
run.

| Task | The thread |
| --- | --- |
| `App` | runs the window of a program built with `--app` |
| `MFB Main` | runs your `main` function |
| `User Thread` | is one you started with `thread::start` |
| `Other` | belongs to the system or the MFBASIC runtime |

In a program built with `--app`, `main` runs on a thread of its own, not on
thread 0. Without `--debug`, `mfb dbg` can recognize that thread only while it
is running your code; while it waits (in `os::sleep`, say) it shows as `Other`.

### Listing registers

`list reg` on its own explains how to list registers. Name a group, or the
registers you want, and it prints them for thread 0, one per line under the
header `Register Hex Decimal`, in hex and then in decimal
(`0x000000000000002d (45)`):

| Group | The registers |
| --- | --- |
| `gpr` | the general-purpose registers: `x0` to `x28`, `fp` (`x29`) and `lr` (`x30`) |
| `simd` | the SIMD and floating-point registers: `v0` to `v31` |
| `sys` | the system registers: `sp`, `pc`, the condition flags `nzcv`, and `fpsr` and `fpcr` |

Put a thread's `<id>` first to see that thread's instead: `list reg 2 gpr`.
Name registers with spaces or commas, `list reg x8, x9, sp`, and mix them with
groups, `list reg sys x0`. A program paused before its first instruction has
almost all of them at zero; `pc` is where it will start.

A function receives its first eight arguments in `x0` to `x7`, in order, and
returns an error code in `x0` (0 when there was none) and its value in `x1`.
A number arrives as itself; a `String`, a collection or a resource arrives as
the address of the value, which `show type` reads (`mfb man debug values`).

### Reading memory

`show mem` prints raw bytes, 64 unless you give a size. Name an address, a
register or a function, each with an offset (`0x16b2d0c00+0x10`, `sp-16`,
`main+0x110`): at `-O0` the values a function works with live at
offsets from `sp`, so `show mem sp+0x10 24` shows three of them at once. Each
line starts with the address, then the bytes in hex, then the ones that are
printable. Numbers are stored lowest byte first, so the 8 bytes
`a7 20 06 00 00 00 00 00` are 0x620a7, 401575.

`show mem` reads any memory the program has. To read a value as its type
instead of as bytes, use `show type` (`mfb man debug values`).

### Reading the current error

When something fails, the error goes in its thread's error slot until a `TRAP`
catches it. `show error` shows the error the current thread has raised and not
yet caught, and `show error 2` thread 2's: its code (with the `errorCode` name
when it is one), its message, and where in your source it was raised.

```
DBG> show error
Thread 0's error in flight (the Error at 0x0000000102550050):
Code: 77050010 (errorCode::ErrOverflow)
Message: "Arithmetic overflow or numeric conversion outside the destination range."
Location: src/main.mfb:11:34
```

Once a `TRAP` has caught the error, the slot is empty again and `show error`
says `Thread 0 has no error in flight.` A good place to look is a breakpoint on
`_mfb_rt_park_error`, the runtime function every failure goes through: after
`finish` the error is in the slot and the `TRAP` has not run yet. `show error`
needs a `--debug` build.

## Examples

Stop in `isPrime`, run up to where it has set its first divisor, then look at
its argument and its stack:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp isPrime\nrun\nstep 7\nlist reg x0, sp, pc\nshow mem sp+0x10 24\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

```
Launched examples/debug/build/debug.out as process 97664; paused before its first instruction.
DBG> bp isPrime
Set a breakpoint at 0x0000000104b2caa4 in isPrime.
DBG> run
--[BP] T: 0 P: 0x0000000104b2caa4 F: isPrime
DBG> step 7
Stepped 7 instructions; thread 0 is at 0x0000000104b2cac0 in isPrime+0x1c.
DBG> list reg x0, sp, pc
Register Hex                Decimal
x0       0x00000000000620a7 (401575)
sp       0x000000016b2d0c00 (6093081600)
pc       0x0000000104b2cac0 (4373793472)
DBG> show mem sp+0x10 24
Memory at 0x000000016b2d0c10 (sp+0x10 of thread 0):
0x000000016b2d0c10 a7 20 06 00 00 00 00 00 a7 20 06 00 00 00 00 00 | .   . . . . . . .   . . . . . .
0x000000016b2d0c20 02 00 00 00 00 00 00 00                         | . . . . . . . .
DBG> q
Killed process 97664.
```

`x0` holds `n`, the number being tested, 401575. `isPrime` keeps it at
`sp+0x10`, a copy at `sp+0x18`, and its first divisor, 2, at `sp+0x20`.

List the functions you wrote; `grep '^MU'` keeps those lines:

```sh macos
mfb build --debug -O0 examples/debug
printf 'list func\nq\n' | mfb dbg examples/debug/build/debug.out | grep '^MU'
```

```
MU   0x00000001009888d0 _mfb_fn_next_5Fseed                                 next_seed
MU   0x0000000100988aa4 _mfb_fn_isPrime                                     isPrime
MU   0x0000000100988d80 _mfb_fn_countPrimes                                 countPrimes
MU   0x0000000100989108 _mfb_fn_runRound                                    runRound
MU   0x00000001009892c4 _mfb_fn_roundWorker                                 roundWorker
MU   0x0000000100989374 _mfb_fn_formatLine                                  formatLine
MU   0x0000000100989824 _mfb_fn_logPath                                     logPath
MU   0x0000000100989ee0 _mfb_fn_summarize                                   summarize
MU   0x000000010098a278 _mfb_fn_runRounds                                   runRounds
MU   0x00000001009904e0 _mfb_fn_main                                        main <mfb>
```

## See also

- `mfb man debug values` — read a value in memory as its type
- `mfb man debug pools` — the memory pools values live in
- `mfb man debug stepping`
