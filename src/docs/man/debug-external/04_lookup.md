# lookup

Turn an address from a crash or a log into a function name, and back

## Description

Sometimes all you have is an address: a crash report, a line a tool printed,
a value you copied out of a debugger. A `--debug` build lets you turn it into
a function name and an offset into that function, and turn a name into its
address.

| To | macOS | Linux | Windows |
| --- | --- | --- | --- |
| list every function and its address | `nm -n <exe>` | `nm -n <exe>` | `nm -n <exe>` |
| name an address in the file | `atos -o <exe> -l 0x100000000 <addr>` | `addr2line -f -e <exe> <addr>` or gdb `info symbol` | gdb `info symbol` |
| name an address in a running program | `atos -p <pid> <addr>` | subtract the load base, then as above | subtract the load base, then as above |

The answer is a name and a byte offset, such as `_mfb_fn_isPrime + 16`: the
address is 16 bytes into `isPrime`. `addr2line` also prints `??:?` on a second
line. That is where a source file and line would go; MFBASIC does not record
them. On Windows `addr2line` prints `??` for the name too, because it cannot
read the names in a Windows executable. Use gdb's `info symbol` there.

### File addresses and live addresses

`nm` prints each function's address **in the file**. Every time the program
starts, the operating system loads it at a different place in memory, so the
address of the same instruction in a **running** program is different. To go
from a live address to a file address, subtract the place the program was
loaded at (its load base) and add the file's own base:

| Platform | File base | Where the load base comes from |
| --- | --- | --- |
| macOS | `0x100000000` | `atos -p` works it out for you. In a crash report, it is the first address on the program's line under *Binary Images*: pass it to `atos -l`. |
| Linux | `0` | the first line naming the program in `/proc/<pid>/maps` |
| Windows | `0x140000000` | the process's `MainModule.BaseAddress` in PowerShell |

A Windows crash recorded in Event Viewer (*Windows Logs › Application*, source
*Application Error*) gives a **fault offset** instead of an address. That
offset is already relative to the load base: the file address is
`0x140000000` plus the offset.

The addresses below come from a `-O0` build, as in the scripts. A different
build places functions at different addresses, so always look an address up
in the exact executable that produced it.

## Examples

Each script builds the example, lists its functions, looks up an address 16
bytes into `isPrime`, then starts a long run, reads the address the program is
at from the debugger, converts it, and looks it up.

macOS:

```sh macos
mfb build --debug -O0 examples/debug
exe=examples/debug/build/debug.out
nm -n "$exe" | grep ' _mfb_fn_'

start=$(nm "$exe" | awk '$3 == "_mfb_fn_isPrime" { print $1 }')
addr=$(printf '0x%x' $(( 0x$start + 0x10 )))
echo "looking up $addr"
atos -o "$exe" -l 0x100000000 "$addr"

"$exe" 3000 20000 > /dev/null 2>&1 &
pid=$!
sleep 1
pc=$(lldb --batch -p "$pid" -o 'p/x $pc' -o 'detach' 2>/dev/null | sed -n 's/^(unsigned long) 0x/0x/p')
echo "live address $pc"
atos -p "$pid" "$pc"
kill "$pid"
```

```
looking up 0x100000ab4
mfb_fn_isPrime (in debug.out) + 16
live address 0x0000000100408ba4
mfb_fn_isPrime (in debug.out) + 256
```

Linux:

```sh linux
mfb build --debug -O0 examples/debug
exe=examples/debug/build/debug-glibc.out
nm -n "$exe" | grep ' _mfb_fn_'

start=$(nm "$exe" | awk '$3 == "_mfb_fn_isPrime" { print $1 }')
addr=$(printf '0x%x' $(( 0x$start + 0x10 )))
echo "looking up $addr"
addr2line -f -e "$exe" "$addr"
gdb -batch -ex "info symbol $addr" "$exe"

"$exe" 3000 20000 > /dev/null 2>&1 &
pid=$!
sleep 1
base=0x$(grep -m1 "$(readlink -f "$exe")" /proc/$pid/maps | cut -d- -f1)
pc=$(gdb -batch -p "$pid" -ex 'thread apply 1 p/x $pc' 2>/dev/null | sed -n 's/.*= //p')
kill "$pid"
file=$(printf '0x%x' $(( pc - base )))
echo "load base $base, live address $pc, file address $file"
gdb -batch -ex "info symbol $file" "$exe"
```

```
looking up 0x161c
_mfb_fn_isPrime
??:?
_mfb_fn_isPrime + 16 in section .text
load base 0xaaaae1bcc000, live address 0xaaaae1bcd83c, file address 0x183c
_mfb_fn_isPrime + 560 in section .text
```

Windows (PowerShell):

```powershell windows
mfb build --debug -O0 examples\debug
$exe = 'examples\debug\build\debug.exe'
nm -n $exe | Select-String ' _mfb_fn_'

$line = nm $exe | Select-String ' _mfb_fn_isPrime$'
$start = [Convert]::ToUInt64(($line -split ' ')[0], 16)
$addr = '0x{0:x}' -f ($start + 0x10)
"looking up $addr"
gdb -batch -ex "info symbol $addr" $exe

$p = Start-Process -FilePath $exe -ArgumentList 3000, 20000 -PassThru -WindowStyle Hidden `
  -RedirectStandardOutput out.txt -RedirectStandardError err.txt
Start-Sleep -Seconds 1
$base = [UInt64]$p.MainModule.BaseAddress.ToInt64()
$out = gdb -batch -p $p.Id -ex 'thread apply 1 p/x $pc' 2>$null | Select-String '= 0x([0-9a-f]+)'
Stop-Process -Id $p.Id
$pc = [Convert]::ToUInt64($out.Matches[0].Groups[1].Value, 16)
$file = '0x{0:x}' -f ($pc - $base + 0x140000000)
'load base 0x{0:x}, live address 0x{1:x}, file address {2}' -f $base, $pc, $file
gdb -batch -ex "info symbol $file" $exe
```

```
looking up 0x1400023f8
_mfb_fn_isPrime + 16 in section .text
load base 0x7ff637ce0000, live address 0x7ff637ce2742, file address 0x140002742
_mfb_fn_isPrime + 858 in section .text
```

## See also

- `mfb man debug-external backtraces` — a debugger names live addresses for you
- `mfb man debug-external`
