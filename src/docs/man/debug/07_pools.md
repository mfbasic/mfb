# pools

The memory pools each thread keeps its values in, and how to look inside one

## Description

```
list arena                                   l arena
show arena [<id>]
show arena <id> <block> [<offset>] [<size>]
```

Each thread that runs MFBASIC code keeps the values it makes — strings, lists,
maps, sets, records, open handles — in a pool of memory of its own;
`list arena` and `show arena` are the commands for them. They need a program
built with `--debug`, and list nothing until it has started running.

### Listing the pools

`list arena` prints one pool per line, under the header
`ID | Thread | Address | Size (KB)`. `ID` is the number `show arena` takes. `Thread`
is the thread that uses the pool (the `ID` from `list thread`), or `-` for a
pool that only carries values from one thread to another. `Size (KB)` is how
much memory the pool holds.

### Looking inside one

`show arena` shows the main thread's pool (`show arena 3` shows pool 3): every
number MFBASIC keeps about the pool, in hex, and then each block of memory the
pool holds, oldest first. A pool starts with one block and adds bigger ones as
your program makes more values:

```
Arena Block (0):
  prevBlock         0x0000000000000000
  blockSize         0x0000000000001000
  usableCapacity    0x0000000000000fe0
  bumpOffset        0x0000000000000000
Arena Block (1):
  prevBlock         0x0000000100a04000
  blockSize         0x0000000000002000
  usableCapacity    0x0000000000001fe0
  bumpOffset        0x0000000000000000
```

`blockSize` is the block's size in bytes, and `usableCapacity` how much of it
holds values. `show arena <id> <block>` prints the block's first 1 KB as raw
bytes, 16 to a line, with the printable ones spelled out at the end of the line;
add an offset and a size (decimal, or hex starting `0x`) to see another part.

The pool fills a new block, and the space a value no longer needs, with random
bytes. Bytes that look like noise are space no value is using; a value stands
out as words of zeros and small numbers, or as text. A pool's bytes do not say
which value they belong to or what type it is: to read a value, find where it
is — in a register, or in the list of a collection's elements — and use
`show type` (`mfb man debug values`).

### Watching memory being asked for

Every value's memory is asked for through `_mfb_arena_alloc` and handed back
through `_mfb_arena_free`, so a breakpoint on either sees each request.
`bp _mfb_arena_alloc list reg x0; finish; run` prints each one: `x0` at the
stop is the number of bytes asked for, and `x1` after the `finish` is where
the pool put them.

The pool rounds every request up to a multiple of 16 bytes, and the figures
in the report (`mfb man debug report`) count the rounded size. A request for
152 bytes stops with `x0` at 152 and adds 160 to the report's `live_bytes`,
so a test must name the size asked for: `bpi x0 eq 152 at _mfb_arena_alloc`,
not 160.

## Examples

Stop in `summarize`, list the pools, and dump part of the main thread's
first block:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp summarize\nrun\nlist arena\nshow arena 0 0 0x340 48\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

```
Launched examples/debug/build/debug.out as process 99522; paused before its first instruction.
DBG> bp summarize
Set a breakpoint at 0x0000000100205ee0 in summarize.
DBG> run
round 1: 37 primes
round 2: 31 primes
--[BP] T: 0 P: 0x0000000100205ee0 F: (summarize) _mfb_fn_summarize
DBG> list arena
ID | Thread | Address            | Size (KB)
0  | 0      | 0x000000016fbf9f50 | 12
DBG> show arena 0 0 0x340 48
Arena Block (0,0):
0x0000_0340 1f 00 00 00 00 00 00 00 2d 96 57 99 20 ed f3 fb | . . . . . . . . - . W .   . . .
0x0000_0350 e1 9a 2f 8a e1 d5 47 3e 6d 06 81 dc 34 5d 62 90 | . . / . . . G > m . . . 4 ] b .
0x0000_0360 02 00 04 01 00 00 00 00 02 00 00 00 00 00 00 00 | . . . . . . . . . . . . . . . .
DBG> q
Killed process 99522.
```

The first word is 31: the second element of `counts`, the `List OF Integer`
`summarize` receives in `x0` (`show type list x0 0..` lists it at this
address). The noise after it is room `counts` keeps for elements it does not
have yet, and the line at `0x0360` is where the next value, `averages`,
begins. With the example's rounds on threads of their own (`3000 20000 1`),
each worker thread has a pool too.

## See also

- `mfb man debug values` — read a value in a pool as its type
- `mfb man debug report` — the pools' figures when the program exits
- `mfb man debug inspecting`
