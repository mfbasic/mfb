# debug

The program the `mfb man debug` walkthroughs debug: it counts the primes in a run
of ranges and prints one line per round plus a total. The output is the same on
every run, and the call chain is a few levels deep, which makes it easy to follow
in a debugger, a profiler and the debug report.

```
main -> runRounds -> runRound -> countPrimes -> isPrime
                              -> next_seed
                  -> formatLine
                  -> roundWorker -> runRound        (threads mode)
```

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

Then read `mfb man debug`, which walks through breakpoints, backtraces,
profiling, address lookup and the debug report on macOS, Linux and Windows.
