# stepping

Run a thread one instruction at a time, move it, and read the machine code it runs

## Description

```
step [<count>] [<thread>]     s …
step-all [<count>]            sa …
disassemble                   dis
dis <count>
dis -<back> <count>
dis <where> [<count>]
dis <function> [<count>]
jump <where> [<register>=<value>]...  j …
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

### Moving a thread

`jump` (`j`) moves the current thread to `<where>`, read the way `show mem`
reads a place (an address, a register or a function, each with an offset), and
can give registers new values on the way: `j <where> x0=5 x1=-1`. A value is a
number, or a place read now: `x1=x2` copies `x2`, and `x2=[sp+8]` takes what is
stored 8 above `sp`. Up to 8 of `x0` to `x28`, `fp`, `lr`, `sp`, `nzcv`,
`fpsr` and `fpcr` can be set; the `pc` is the place itself. Every value is read
before any is set, and the thread stays paused, so `run` or `step` carries it
on from there:

```
DBG> j lr x0=0 x1=7
Moved thread 0 to 0x00000001006ed284 in runRound+0x158; set x0 = 0x0000000000000000 (0), x1 = 0x0000000000000007 (7).
```

At a function's first instruction, `lr` holds the place it returns to and the
function has done nothing yet, so `j lr x0=0 x1=7` there makes it return at
once: no error in `x0`, and 7 as its value in `x1`. Past its first instruction
a function has set up its own values below `sp`, and leaving it by `lr` skips
putting them back, so what the program does next is hard to predict. A thread
moved onto a breakpoint runs past it the next time the program runs, as it does
whenever it is paused on one.

### Reading machine code

`dis` shows the machine code of the function the current thread is in, the
way lldb does; `->` marks the instruction the thread runs next. `dis 5` shows
just the next five instructions, and `dis -3` the three before it and then
that one, to see how the thread got there:

```
DBG> dis -3
debug.out`isPrime:
    0x00000001049c4aa4 <+0>:  sub    sp, sp, #0xf0
    0x00000001049c4aa8 <+4>:  str    x30, [sp]
    0x00000001049c4aac <+8>:  str    x0, [sp, #0x10]
->  0x00000001049c4ab0 <+12>: ldr    x8, [sp, #0x10]
```

To see both sides, give two counts: `dis -6 8` starts six instructions
before the current one and shows eight, so the current one and the one after
it too. When the instructions shown run into another function, its name heads
them, so the end of the function before this one is not mistaken for this
one's.

`dis isPrime` or `dis exit` shows a function you name, the same way
`breakpoint` reads a name, and `dis isPrime 3` its first three instructions.
`dis` also starts anywhere you name an address, a register or a function, each
with an offset: `dis pc-8 4` shows four instructions from two before the
current one, `dis lr 2`, at a breakpoint on a function's first instruction, the
code it returns to, `dis 0x1049c4ab0+0x40` the code at that address, and
`dis main+0x110 4` the code at a place `bt` or a stop showed you. A negative
count, or two counts, works with all of them: `dis isPrime -2 4`. A count that
runs back past the start of the function into code that is in no function
shows those addresses as `<-8>`, before the function. Every instruction is 4 bytes, so an address that
is not a multiple of 4 is refused.

The notes after `;` say where a branch goes (`<+68>` is 68 bytes into this
function), which function a call reaches, where an address an instruction
names points, and the value of a number. Your functions are shown in full only
in a `--debug` build; otherwise, and for code the debugger knows nothing about,
it shows up to the first return or 64 instructions. `mfb dbg` reads the
instructions of MFBASIC programs and of the system libraries they call, written
as lldb writes them. The few it does not know, such as the scalable vector
instructions some system libraries use, show as `.word` and their number.

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
--[BP] T: 0 P: 0x0000000102734aa4 F: isPrime
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

Make the first round's `countPrimes` return 7 without counting anything, then
let the rest of the program run:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp countPrimes\nrun\nj lr x0=0 x1=7\nbp r countPrimes\nrun\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

```
Launched examples/debug/build/debug.out as process 82633; paused before its first instruction.
DBG> bp countPrimes
Set a breakpoint at 0x00000001006ecdc4 in countPrimes.
DBG> run
--[BP] T: 0 P: 0x00000001006ecdc4 F: countPrimes
DBG> j lr x0=0 x1=7
Moved thread 0 to 0x00000001006ed284 in runRound+0x158; set x0 = 0x0000000000000000 (0), x1 = 0x0000000000000007 (7).
DBG> bp r countPrimes
Removed the breakpoint at 0x00000001006ecdc4 in countPrimes.
DBG> run
round 1: 7 primes
round 2: 31 primes
total: 38 primes
--[EX] Program exit (0)
DBG> q
Process 82633 had already exited.
```

At `-O0`, each value a function works with has a place of its own below `sp`:
here `isPrime` copies its argument `n` from `sp+0x10` to `sp+0x18`, then puts
2, the first divisor it tries, at `sp+0x20`. `mfb man debug inspecting` shows
how to read them.

## See also

- `mfb man debug breakpoints`
- `mfb man debug inspecting`
- `mfb man debug backtraces`
