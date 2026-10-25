# riscv64

Debug on a RISC-V (riscv64) Linux computer: what differs from an Arm one

## Description

`mfb dbg` works on a RISC-V computer running Linux the way the other pages of
this topic show it working on Arm: the same commands, the same listings, the
same scripts. What differs is the processor. Its registers have other names,
its instructions are 2 or 4 bytes long, and Linux on RISC-V gives a debugger
no watchpoints. Wherever another page shows an Arm register, this page says
which RISC-V register to read instead.

### Registers

A RISC-V register goes by the name its calling convention gives it, and only
by that name: `a0`, `ra`, `s0`, `fa0`. The numbered names (`x10`, `f10`) and
`fp` name nothing in `mfb dbg`, in `list reg`, in a `<where>`, or in `jump`.

`list reg` shows them in three groups:

| Group | Registers |
| --- | --- |
| `gpr` | `ra`, `gp`, `tp`, `t0` to `t6`, `s0` to `s11`, `a0` to `a7` |
| `sys` | `sp`, `pc`, `fcsr` |
| `simd` | the floating-point registers: `ft0` to `ft11`, `fs0` to `fs11`, `fa0` to `fa7` |

`zero`, which always reads 0, is not listed. Every register is 64 bits, so
every value shows as 16 hex digits.

### The registers a call uses

`list reg` on its own shows the registers a call uses, each with its role,
the same roles the Arm registers on the other pages have:

| Register | Role | On Arm |
| --- | --- | --- |
| `a0`, `a1` | `MFB arg, MFB result, C arg, C result` | `x0`, `x1` |
| `a2`, `a3` | `MFB arg, MFB result, C arg` | `x2`, `x3` |
| `a4` to `a7` | `MFB arg, C arg` | `x4` to `x7` |
| `s11` | `Arena` | `x19` |
| `s2` | `Current Thread` | `x20` |
| `s10` | `Closure` | `x28` |

A function receives its first eight values in `a0` to `a7`, in order, so
where another page reads `x0` and `x1`, read `a0` and `a1`: `show type list a0
0..` at a stop in `summarize` shows the list it was handed. A function that
returns gives back an error code in `a0` (0 when there was none) and its value
in `a1`; `finish` prints both.

The return address is in `ra`, where Arm has it in `lr`: at a function's first
instruction, before it has saved anything, `j ra a0=0 a1=7` returns from it at
once with no error and the value 7.

### Instructions

An instruction is 4 bytes, or 2 for the short (compressed) form of a common
one, so the addresses `dis` lists do not always go up by 4, and `dis -6 8`
counts six instructions back, whatever their sizes. A short instruction is
shown as the full instruction it stands for. The text is the one GNU `objdump`
prints (`addi sp,sp,-208`, `sd ra,0(sp)`, `ret`), so a listing reads the
same as one from the system's own tools. An instruction `mfb dbg` does not know
shows as `.word` (4 bytes) or `.short` (2).

`jump` and `dis <where>` take any address where an instruction can start: a
multiple of 2.

### No watchpoints

Linux on RISC-V gives a debugger no way to stop a thread as it changes memory,
so `wp` says so and sets nothing:

```
DBG> wp sp 8
error: this system has no watchpoints: the riscv64 kernel does not provide them
```

To find what changes a value, put a breakpoint on each function that can
change it and look at the value with `show mem` (or `show type`) at every
stop; `bp <function> show mem <where> 8; run` does that without stopping
(`mfb man debug breakpoints`).

### Stepping

`step` runs one instruction of a thread, as on Arm. `step-all` runs every
thread one instruction per round too, but within a round the threads take
their turns one after another instead of all at once; each still runs the same
instructions.

## Examples

A session on a RISC-V computer running Alpine Linux (musl, so
`debug-musl.out`): stop in `isPrime`, show the registers a call uses, the
calls that led there, and what `isPrime` returns:

```
mfb build --debug -O0 examples/debug
cd examples/debug/build
printf 'bp isPrime\nrun\nlist reg\nbt\nfinish\nq\n' | mfb dbg debug-musl.out -- 2 500
```

```
Launched debug-musl.out as process 18812; paused before its first instruction.
DBG> bp isPrime
Set a breakpoint at 0x000055558842f6b0 in isPrime.
DBG> run
--[BP] T: 0 P: 0x000055558842f6b0 F: (isPrime) _mfb_fn_isPrime
DBG> list reg
Register | Hex                | Decimal                | Role
a0       | 0x00000000000620a7 | (401575)               | MFB arg, MFB result, C arg, C result
a1       | 0x00000000000001f4 | (500)                  | MFB arg, MFB result, C arg, C result
a2       | 0x0000000000000001 | (1)                    | MFB arg, MFB result, C arg
a3       | 0x0000000000000180 | (384)                  | MFB arg, MFB result, C arg
a4       | 0x8024100000000000 | (-9213221345507147776) | MFB arg, C arg
a5       | 0xfffffffffffff000 | (-4096)                | MFB arg, C arg
a6       | 0x0000000000000038 | (56)                   | MFB arg, C arg
a7       | 0x0000000000000019 | (25)                   | MFB arg, C arg
s2       | 0x00007fffdb701e70 | (140736874946160)      | Current Thread
s10      | 0x0000000000000000 | (0)                    | Closure
s11      | 0x00007fffdb700fa0 | (140736874942368)      | Arena

Usage: list reg [<id>] [<what>...]

Shows the registers of thread <id> (default 0, the main thread) as
<register> <hex value> (<decimal>). With no <what> it shows the registers
that carry arguments and results, and the ones MFBASIC code keeps for
itself, each with its role. <what> is one or more of:

  gpr           The general-purpose registers: ra, gp, tp, t0 to t6, s0 to
                s11, a0 to a7
  simd          The floating-point registers: ft0 to ft11, fs0 to fs11,
                fa0 to fa7
  sys           The system registers: sp, pc, fcsr
  <register>    One register by name; list several with spaces or commas,
                as in: list reg a0, a1, sp
DBG> bt
* thread #0, task = 'MFB Main'
  * frame #0: 0x000055558842f6b0 debug-musl.out`isPrime
    frame #1: 0x000055558842fb6c debug-musl.out`countPrimes + 504
    frame #2: 0x000055558842fed4 debug-musl.out`runRound + 368
    frame #3: 0x00005555884329ac debug-musl.out`runRounds + 6280
    frame #4: 0x000055558843cd00 debug-musl.out`main + 2896
    frame #5: 0x000055558842f2d0 debug-musl.out`_main + 696
DBG> finish
Thread 0 returned from isPrime to 0x000055558842fb6c in countPrimes+0x1f8: a0 = 0x0000000000000000 (0), a1 = 0x0000000000000000 (0).
DBG> q
Killed process 18812.
```

`isPrime` takes one `Integer`, so `a0` holds the number it was handed,
401575; the other argument registers hold whatever they held before the call.
It returns no error (`a0 = 0`) and `FALSE` (0 in `a1`): 401575 is not prime.
The addresses change from run to run.

## See also

- `mfb man debug start` — starting and attaching, and the `-glibc` and `-musl` executables
- `mfb man debug inspecting` — `list reg` and reading memory
- `mfb man debug stepping` — `step`, `dis` and `jump`
- `mfb man debug breakpoints` — breakpoints, and commands at a stop
- `mfb spec tooling debugger` — the exact behavior of `mfb dbg`
