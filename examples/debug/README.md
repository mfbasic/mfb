# debug

The program the `mfb man debug` and `mfb man debug-external` walkthroughs debug: it
counts the primes in a run of ranges and prints one line per round plus a total. The
output is the same on every run, and the call chain is a few levels deep, which makes
it easy to follow in a debugger, a profiler and the debug report.

```
main -> runRounds -> runRound -> countPrimes -> isPrime
                              -> next_seed
                  -> formatLine
                  -> summarize
                  -> roundWorker -> runRound        (threads mode)
```

`runRounds` keeps what each round found in one value of each kind `mfb dbg` can show
with `show type` — a `List`, a `Map`, a `Set`, and lists of `Float`, `Fixed` and
`Money` — and writes each line to a log file it holds open (in the temporary
directory, removed at the end). `summarize` receives all of them, so a breakpoint
there sees them side by side.

## Build and run

```sh
mfb build --debug -O0 examples/debug
./examples/debug/build/debug.out            # macOS
./examples/debug/build/debug-glibc.out      # Linux (or debug-musl.out)
examples\debug\build\debug.exe              # Windows
```

Arguments, all optional: `rounds` (default 5), `size` (default 20000), and
`threads` (`1` runs each round on its own worker thread). `3000 20000` runs for
about ten seconds, long enough to attach a debugger or a profiler.

Then read `mfb man debug`, which walks through `mfb dbg`: breakpoints, stepping,
backtraces, registers, values in memory and the debug report. `mfb man debug-external`
does the same with lldb, gdb and each platform's profiler and address-lookup tools.

## Debug it

```sh
printf 'bp summarize\nrun\nbt\nshow type map x1 keys\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500
```
