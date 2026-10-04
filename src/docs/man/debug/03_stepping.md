# stepping

Run a thread one instruction at a time, and read the machine code it runs

## Description

```
step [<count>] [<thread>]     s …
step-all [<count>]            sa …
disassemble                   dis
dis <count>
dis <function>
```

`step` runs one instruction of thread 0, the main thread of a console program,
and pauses it again; `step 10` runs ten, and `step 10 2` runs ten of thread 2
(the `<id>` from `list thread`). Every other thread stays paused. It then says
where the thread is:

```
DBG> step 3
Stepped 3 instructions; thread 0 is at 0x0000000102734ab0 in isPrime+0xc.
```

`step 0` only says where the thread is, and a negative count is refused.

A thread waiting in the system, in `os::sleep` or `thread::waitFor` for
example, cannot run its next instruction until the wait ends. `step` notices
that the thread is waiting in the system, gives up after two seconds, and says
so:

```
Stopped after 0 instructions: thread 0 is waiting in a system call at 0x00000001967f53cc.
```

`step-all` runs every thread together, one instruction each per round, and
says where each one stopped. A thread that is waiting sits out the rest of the
rounds while the others carry on. Here the main thread waits for the worker
that `thread::start` began, and the worker runs three instructions:

```
DBG> sa 3
Thread 0: stopped after 0 instructions, waiting in a system call; at 0x00000001967f53cc.
Thread 1: stepped 3 instructions; at 0x0000000100424b8c in isPrime+0xe8.
```

### The current thread

`dis`, `bt` and the registers `show mem` and `show type` read belong to the
current thread: the one `run` last stopped or `step` last ran, thread 0 at
first.

### Reading machine code

`dis` shows the machine code of the function the current thread is in, the
way lldb does; `->` marks the instruction the thread runs next. `dis 5` shows
just the next five instructions, and `dis isPrime` or `dis exit` shows a
function you name, the same way `breakpoint` reads a name.

The notes after `;` say where a branch goes (`<+68>` is 68 bytes into this
function), which function a call reaches, and the value of a number. Your
functions are shown in full only in a `--debug` build; otherwise, and for code
the debugger knows nothing about, it shows up to the first `ret` or 64
instructions. `mfb dbg` reads the instructions MFBASIC programs are made of; an
instruction it does not know, which happens mostly inside system libraries,
shows as `.word` and its number.

## Examples

Stop in `isPrime`, run three of its instructions, and show the next five:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp isPrime\nrun\nstep 3\ndis 5\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

```
Launched examples/debug/build/debug.out as process 89969; paused before its first instruction.
DBG> bp isPrime
Set a breakpoint at 0x0000000102734aa4 in isPrime.
DBG> run
Thread 0 hit a breakpoint at 0x0000000102734aa4 in isPrime.
DBG> step 3
Stepped 3 instructions; thread 0 is at 0x0000000102734ab0 in isPrime+0xc.
DBG> dis 5
debug.out`isPrime:
->  0x0000000102734ab0 <+12>: ldr    x8, [sp, #0x10]
    0x0000000102734ab4 <+16>: str    x8, [sp, #0x18]
    0x0000000102734ab8 <+20>: mov    x8, #0x2                  ; =2
    0x0000000102734abc <+24>: str    x8, [sp, #0x20]
    0x0000000102734ac0 <+28>: ldr    x8, [sp, #0x18]
DBG> q
Killed process 89969.
```

At `-O0`, each value a function works with has a place of its own below `sp`:
here `isPrime` copies its argument `n` from `sp+0x10` to `sp+0x18`, then puts
2, the first divisor it tries, at `sp+0x20`. `mfb man debug inspecting` shows
how to read them.

## See also

- `mfb man debug breakpoints`
- `mfb man debug inspecting`
- `mfb man debug backtraces`
