# profiler

Find the functions the program spends its time in

## Description

A sampling profiler stops the program many times a second, notes which
function it is in, and lets it carry on. The functions that show up most often
are where the time goes, and they are where a speedup pays off.

| Platform | Profiler |
| --- | --- |
| macOS | `sample`, part of the Xcode Command Line Tools. |
| Linux, Windows | a short gdb loop: attach, note frame `#0`, detach, repeat. |

The gdb loop takes a few samples a second, far fewer than `sample` takes, but a
function that holds most of the time shows up in most samples. Take more
samples for a closer picture.

**Profile the default build**: `mfb build --debug`, without `-O0`. That is the
code your program really runs. A small function the optimizer merged into its
caller is counted as part of that caller.

The program has to be running while you sample it, so give it work that lasts
a few seconds. `debug 3000 20000` runs for about ten seconds.

### Reading the result

The example spends almost all of its time testing numbers, and every profile
below says so: nearly every sample is in `isPrime`. A few land in
`countPrimes`, the loop that calls it. A sample that reads `??` landed in code
that has no name, such as a system library built without names. Count it as
time spent outside your program.

When the top function is your own, speed up that function or call it less.
When it is a runtime function (`_mfb_rt_…`, `_mfb_arena_…`), look at what
your code asks of it: building strings in a loop, for example. Whichever
function the samples point at, a backtrace (`mfb man debug backtraces`) taken
while the program is in it shows which of your functions are calling it.

## Examples

Each script builds the example, starts a ten-second run in the background,
waits a second for it to get going (so no samples land in the program's
start-up), samples it, stops it, and prints how many samples landed in each
function.

macOS. `sample <pid> 3` samples for three seconds, about a thousand times a
second, and writes a report. Its *Sort by top of stack* section is the
summary:

```sh macos
mfb build --debug examples/debug
examples/debug/build/debug.out 3000 20000 > /dev/null 2>&1 &
pid=$!
sleep 1
sample "$pid" 3 -file debug.sample > /dev/null 2>&1
kill "$pid"
sed -n '/Sort by top of stack/,/^$/p' debug.sample
```

```
Sort by top of stack, same collapsed (when >= 5):
        mfb_fn_isPrime  (in debug.out)        2422
        mfb_fn_countPrimes  (in debug.out)        35
```

The full report in `debug.sample` also has a *Call graph* section, which
nests each function under its caller with a sample count at each level.

Linux. The loop attaches gdb twenty times, keeps the frame `#0` line, and
counts the function names:

```sh linux
mfb build --debug examples/debug
examples/debug/build/debug-glibc.out 3000 20000 > /dev/null 2>&1 &
pid=$!
sleep 1
for i in $(seq 1 20); do
  gdb -batch -p "$pid" -ex 'thread apply 1 bt 1' 2>/dev/null | grep '^#0'
done | sed -E 's/^#0 +(0x[0-9a-f]+ in )?([^ ]+) .*/\2/' | sort | uniq -c | sort -rn
kill "$pid"
```

```
     20 _mfb_fn_isPrime
```

A run can also show a line or two of `??` (see *Reading the result*).

Windows (PowerShell). The same loop, counting with `Group-Object`:

```powershell windows
mfb build --debug examples\debug
$p = Start-Process -FilePath examples\debug\build\debug.exe -ArgumentList 3000, 20000 `
  -PassThru -WindowStyle Hidden -RedirectStandardOutput out.txt -RedirectStandardError err.txt
Start-Sleep -Seconds 1
$samples = foreach ($i in 1..20) {
  gdb -batch -p $p.Id -ex 'thread apply 1 bt 1' 2>$null |
    Select-String '^#0' |
    ForEach-Object { if ($_ -match ' in (\S+) ') { $Matches[1] } }
}
$samples | Group-Object | Sort-Object Count -Descending | Format-Table Count, Name -AutoSize
Stop-Process -Id $p.Id
```

```
Count Name
----- ----
   20 _mfb_fn_isPrime
```

`thread apply 1` samples the thread running your program. On Windows, gdb
otherwise reports the extra thread that attaching starts.

## See also

- `mfb man debug backtraces`
- `mfb man debug report` — the program's own timings and memory figures at exit
- `mfb man optimizations`
