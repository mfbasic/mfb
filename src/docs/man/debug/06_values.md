# values

Show a value in memory as the type it is: a String, a List, a Map, a Set, a number, a File

## Description

```
show type                                   the types and their forms
show type <type> <where>                    one value
show type list <where> [<type>] <index>     a list's elements: 3, 0..10, 2..
show type set <where> [<type>] <index>      a set's elements
show type map <where> keys | values | <index> | get <key>
```

Memory does not say what type of value it holds: a `String` is its length in
8 bytes, then its text, with nothing to mark it as a `String`. When you know
what is at an address, `show type` reads it as that type.

`<where>` is an address, or a register or `[…]` holding one, as for
`show mem`: the
place in memory where the value is. A function receives its first eight
arguments in `x0` to `x7`, and a `String`, a collection or a resource arrives
as the address of the value, so at a breakpoint at the start of a function
`show type list x0` reads its first argument. A function returning one leaves
its address in `x1`, as `finish` shows.

`show type` reads only your program's values, which live in its memory pools
(`mfb man debug pools`), so it needs a `--debug` build, and an address outside
every pool is refused; `show mem` reads any memory. So is memory that cannot be
the type you name: text that is not UTF-8, a list whose sizes do not add up, a
resource whose kind is unknown. Text from your source (a literal such as
`"total: "`) is kept in the program file, not a pool, but `show type string`
reads it there too and says which literal it is:
`Length: 17 Value: "function handler " (the literal mfb_str_2)`.

### Numbers and text

| Command | Shows |
| --- | --- |
| `show type int <where>` | `Value: 0xffffffffffffffec (-20)` |
| `show type float <where>` | `Value: 0x400c000000000000 (3.5)` |
| `show type fixed <where>` | `Value: 0x0000000180000000 (1.50)` |
| `show type money <where>` | `Value: 0x000000000001e848 (1.25000)` |
| `show type bool <where>` | `Value: 0x01 (TRUE)` |
| `show type byte <where>` | `Value: 0x41 (65)` |
| `show type scalar <where>` | `Value: 0x000000e9 (U+00E9 'é')` |
| `show type string <where>` | `Length: 18 Value: "round 1: 37 primes"` |

Each shows the bytes in hex, then the value: a `Fixed` the way `toString`
prints it, a `Money` with all five decimal places. The MFBASIC names work too:
`show type Integer x1`. A number is kept in a pool as part of a collection, so
the usual way to reach one is through the collection; each element's address
is listed with it, ready to read with the number's type.

### Lists, sets and maps

A list, set or map shows its header: the type of its elements, how many there
are, how many it has room for, how much of its data space is used, and where
the whole block is. Give an index, or a range (`0..10` stops before 10, `2..`
runs to the end), for its elements, each with its address and size:

```
DBG> show type list x0
List Type: Integer Length: 2 Capacity: 4 Data: 16/32 bytes Block: 0x00000001048b4310-0x00000001048b4358 (72 bytes)
DBG> show type list x0 0..
Index | Address            | Bytes | Value
0     | 0x00000001048b4338 | 8     | 0x0000000000000025 (37)
1     | 0x00000001048b4340 | 8     | 0x000000000000001f (31)
```

A list knows the type of its elements except for a `Set`, a resource, a record,
an enum or a function, which it shows as `object`; name the type before the
index to read those: `show type list <where> resource 0..`. A list inside a list
shows its own header, and its address, to read further.

A set shows its elements the same way. Removing from a set or a map can leave
the removed entry in place until it is tidied up; the header counts them as
`Removed`, and the listings skip them. A map shows its keys, its values, one
entry by index, or the entry for a key:

```
DBG> show type map x1 keys
Index | Address            | Bytes | Key
0     | 0x00000001048b4528 | 7     | "round 1"
1     | 0x00000001048b4538 | 7     | "round 2"
DBG> show type map x1 get round 2
Index: 1
Key: "round 2" at 0x00000001048b4538 (7 bytes)
Value: 0x000000000000001f (31) at 0x00000001048b4540 (8 bytes)
```

The key after `get` is the rest of the line, so it can hold spaces; quotes
around it are optional. A number key is written as you would write the number.

### Resources

`show type resource <where>` shows an open handle such as a `File`: its kind,
whether it is closed, whether it was handed to another thread (`Moved`), its
system handle, its `STATE`, the scope that closes it, and the fields of its
kind, such as a file's buffers:

```
DBG> show type resource x6
Type: File Closed: N Moved: N Handle (fd): 3 State: 0x0000000000000000 Scope: 0x16b5c5087fff (scope depth 1 of the frame whose token slot is 0x16b5c5088) Write Buffer: 0x0 Write Filled: 0 Write Enabled: 0 Read Buffer: 0x0 Read Position: 0 Read Filled: 0 At EOF: 0
```

`Scope` is `none` for a handle no binding holds yet, such as one a function is
about to return.

Records, enums, unions and `Result` values cannot be shown by type yet; read
them with `show mem`.

## Examples

`summarize` receives one value of each kind. Stop there and read them all:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp summarize\nrun\nshow type list x0\nshow type map x1 keys\nshow type set x2 0..\nshow type list x3 0..\nshow type list x4 1\nshow type list x5 0\nshow type resource x6\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500
```

```
Launched examples/debug/build/debug.out as process 98307; paused before its first instruction.
DBG> bp summarize
Set a breakpoint at 0x0000000104839ee0 in summarize.
DBG> run
round 1: 37 primes
round 2: 31 primes
--[BP] T: 0 P: 0x0000000104839ee0 F: (summarize) _mfb_fn_summarize
DBG> show type list x0
List Type: Integer Length: 2 Capacity: 4 Data: 16/32 bytes Block: 0x00000001048b4310-0x00000001048b4358 (72 bytes)
DBG> show type map x1 keys
Index | Address            | Bytes | Key
0     | 0x00000001048b4528 | 7     | "round 1"
1     | 0x00000001048b4538 | 7     | "round 2"
DBG> show type set x2 0..
Index | Address            | Bytes | Element
0     | 0x00000001048b4658 | 8     | 0x0000000000000025 (37)
1     | 0x00000001048b4668 | 8     | 0x000000000000001f (31)
DBG> show type list x3 0..
Index | Address            | Bytes | Value
0     | 0x00000001048b4388 | 8     | 0x4042800000000000 (37.0)
1     | 0x00000001048b4390 | 8     | 0x4041000000000000 (34.0)
DBG> show type list x4 1
Index | Address            | Bytes | Value
1     | 0x00000001048b4710 | 8     | 0x0000000633333333 (6.20)
DBG> show type list x5 0
Index | Address            | Bytes | Value
0     | 0x00000001048b4758 | 8     | 0x00000000000001f4 (0.00500)
DBG> show type resource x6
Type: File Closed: N Moved: N Handle (fd): 3 State: 0x0000000000000000 Scope: 0x16b5c5087fff (scope depth 1 of the frame whose token slot is 0x16b5c5088) Write Buffer: 0x0 Write Filled: 0 Write Enabled: 0 Read Buffer: 0x0 Read Position: 0 Read Filled: 0 At EOF: 0
DBG> q
Killed process 98307.
```

`x0` to `x6` are `counts`, `byRound`, `distinct`, `averages`, `shares`, `costs`
and `log`, in the order `summarize` takes them: the running average after each
round, the percentage of each range that is prime, and what each round cost.

## See also

- `mfb man debug inspecting` — registers and raw memory
- `mfb man debug pools` — where these values live
- `mfb man debug breakpoints` — `finish` and the value a function returns
