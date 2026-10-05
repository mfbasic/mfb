# backtraces

See the calls that led to where each thread is

## Description

```
backtrace [<count>] [<thread>]     bt …
backtrace all                      bt all
```

`bt` shows the calls that led to where the program is, the way lldb's `bt`
does: the function the current thread is in first, then the function that
called it, and so on out to your program's entry. The current thread is the
one `run` last stopped or `step` last ran (thread 0 at first).

```
DBG> bt
* thread #0, task = 'MFB Main'
  * frame #0: 0x0000000100d9caa4 debug.out`isPrime
    frame #1: 0x0000000100d9cfc8 debug.out`countPrimes + 584
    frame #2: 0x0000000100d9d260 debug.out`runRound + 344
    frame #3: 0x0000000100d9f678 debug.out`runRounds + 5120
    frame #4: 0x0000000100da5040 debug.out`main + 2912 <mfb>
    frame #5: 0x0000000100d9c634 debug.out`main + 348 <native>
```

Each frame is the address the thread will go on from in that function — for
every frame but the first, the instruction after the call — then the file and
function, and how many bytes into the function that is. Functions are named as
`dis` names them. Your `main` and the program's native entry, which calls it,
are both named `main`, so each carries a tag: `<mfb>` for MFBASIC code (yours or
a built-in package's), `<native>` for the runtime's or a system library's.
Every listing tags a name that way when MFBASIC and native code share it, such
as your own `exit` and the C library's.

`bt 3` shows only the first three frames, `bt 3 2` the first three of thread 2
(the `<id>` from `list thread`), and `bt all` every thread's, the current one
marked `*`. A thread paused in a system library shows the library's functions
too, the way lldb shows them, and a built-in function the runtime carries out
for you is named the way you call it: `thread::waitFor`.

## Examples

Stop in `isPrime` and see how the program got there:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp isPrime\nrun\nbt\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

The output is the backtrace above. With each round on its own thread, every
thread has calls of its own. Attach while the example runs:

```sh macos
mfb build --debug -O0 examples/debug
examples/debug/build/debug.out 3000 20000 1 > rounds.txt 2> report.txt &
sleep 1
printf 'bt all\nq\n' | mfb dbg $!
kill $!
```

```
Attached to process 90091 (/Users/you/mfb/examples/debug/build/debug.out); paused.
DBG> bt all
* thread #0, task = 'MFB Main'
  * frame #0: 0x00000001967f53cc libsystem_kernel.dylib`__psynch_cvwait + 8
    frame #1: 0x000000019683409c libsystem_pthread.dylib`_pthread_cond_wait + 984
    frame #2: 0x00000001004335b0 debug.out`thread::waitFor + 96
    frame #3: 0x000000010042715c debug.out`runRounds + 3812
    frame #4: 0x000000010042d040 debug.out`main + 2912 <mfb>
    frame #5: 0x0000000100424634 debug.out`main + 348 <native>
  thread #1, task = 'User Thread'
    frame #0: 0x0000000100424b8c debug.out`isPrime + 232
    frame #1: 0x0000000100424fc8 debug.out`countPrimes + 584
    frame #2: 0x0000000100425260 debug.out`runRound + 344
    frame #3: 0x0000000100425310 debug.out`roundWorker + 76
    frame #4: 0x00000001004355ac debug.out`mfb_rt_thread_trampoline + 132
    frame #5: 0x0000000196833bc8 libsystem_pthread.dylib`_pthread_start + 136
    frame #6: 0x000000019682eb80 libsystem_pthread.dylib`thread_start + 8
DBG> q
Detached from process 90091; it continues running.
```

The main thread is waiting in `thread::waitFor` for the worker, which is
counting primes in `isPrime`. The worker's calls start in the system and in the
runtime function that starts every thread.

## See also

- `mfb man debug breakpoints`
- `mfb man debug stepping`
- `mfb man debug-external backtraces` — backtraces in lldb and gdb
