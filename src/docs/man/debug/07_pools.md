# pools

The memory pools each thread keeps its values in, and how to look inside one

## Description

```
list arena                                   l arena
show arena [<id>]
show arena <id> <region> [<offset>] [<size>]
```

Each thread that runs MFBASIC code keeps the values it makes — strings, lists,
maps, sets, records, open handles — in a pool of memory of its own;
`list arena` and `show arena` are the commands for them. They need a program
built with `--debug`, and list nothing until it has started running.

### Listing the pools

`list arena` prints one pool per line, under the header
`ID | Thread | Address | Committed (KB)`. `ID` is the number `show arena` takes.
`Thread` is the thread that uses the pool (the `ID` from `list thread`), or `-`
for a pool that only carries values from one thread to another. `Committed (KB)`
is how much memory the pool holds from the operating system. A pool takes memory
a few megabytes at a time, so even a small program's pool holds several.

### Looking inside one

`show arena` shows the main thread's pool (`show arena 3` shows pool 3): every
number MFBASIC keeps about the pool, in hex, then its three regions and the free
space it is keeping for reuse. Values of 2048 bytes or less live in region 0,
`small`; larger ones in region 1, `medium`; values of 32 MiB or more in region 2,
`huge`:

```
Arena Region (0 small):
  start             0x0000007001680000
  handedOut         0x00000000000a0000
  committed         0x0000000000180000
Arena Region (1 medium):
  start             0x00000070b1800000
  handedOut         0x0000000000003ee0
  committed         0x0000000000400000
Arena Region (2 huge):
  start             0x0000007500000000
  handedOut         0x0000000000000000
  committed         0x0000000000000000
Free Runs: 1
  medium[1]         0x00000070b1803310 0x0000000000000bd0
```

`handedOut` is how much of the region the pool has used so far, and `committed`
how much of it holds memory from the operating system. Each free run is space a
value gave back, listed with its address and size, ready for the next value that
fits. `show arena <id> <region>` prints the region's first 1 KB as raw bytes, 16
to a line, with the printable ones spelled out at the end of the line; add an
offset and a size (decimal, or hex starting `0x`) to see another part.

Space no value is using reads zero: the pool overwrites a value's bytes with zeros
when the value is done with them, and new memory from the operating system starts
out zero. A pool's bytes do not say which value they belong to or what type it is:
to read a value, find where it is — in a register, or in the list of a
collection's elements — and use `show type` (`mfb man debug values`).

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

Stop in `summarize`, list the pools, and dump the part of the main thread's
small region that holds `counts`:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp summarize\nrun\nlist arena\nshow arena 0 0 0x60020 48\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500
```

```
Launched examples/debug/build/debug.out as process 73670; paused before its first instruction.
DBG> bp summarize
Set a breakpoint at 0x0000000102c69e5c in summarize.
DBG> run
round 1: 37 primes
round 2: 31 primes
--[BP] T: 0 P: 0x0000000102c69e5c F: (summarize) _mfb_fn_summarize
DBG> list arena
ID | Thread | Address            | Committed (KB)
0  | 0      | 0x000000016d195ed0 | 5824
DBG> show arena 0 0 0x60020 48
Arena Region (0,0 small):
0x0006_0020 20 00 00 00 00 00 00 00 25 00 00 00 00 00 00 00 |   . . . . . . . % . . . . . . .
0x0006_0030 1f 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 | . . . . . . . . . . . . . . . .
0x0006_0040 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 | . . . . . . . . . . . . . . . .
DBG> q
Killed process 73670.
```

`counts`, the `List OF Integer` `summarize` receives in `x0`, starts at offset
`0x60000` of the small region (`list reg x0` shows its address, and `show type
list x0 0..` lists it). The line at `0x6_0020` ends its header, then come its two
elements, 37 and 31; the zeros after them are room `counts` keeps for elements it
does not have yet. Where the dump lands on a different run depends on the program
and its input. With the example's rounds on threads of their own (`3000 20000 1`),
each worker thread has a pool too.

## See also

- `mfb man debug values` — read a value in a pool as its type
- `mfb man debug report` — the pools' figures when the program exits
- `mfb man debug inspecting`
