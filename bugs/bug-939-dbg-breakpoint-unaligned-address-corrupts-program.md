# bug-939: `mfb dbg` accepts a breakpoint at an address that is not a multiple of 4 and corrupts two instructions

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: src/debugger/breakpoints.rs unit test (new), beside the `set` tests; plus a `cli_dbg::macos` case

`breakpoint <target>` takes an address, or a place `<name>+<offset>`, and writes
`brk #0` (4 bytes) there without checking that the address starts an AArch64
instruction. All A64 instructions are 4 bytes and 4-aligned. A breakpoint at
`count+2` therefore overwrites the upper half of one instruction and the lower half
of the next. The program then dies on an illegal instruction instead of stopping at
a breakpoint. The debugger reports this as an ordinary exception (`--[XC] … X: an
illegal instruction`), so the user gets no hint that the breakpoint caused it.

`dis` already refuses the same addresses: `0x… is not the start of an instruction:
they start every 4 bytes`. **Correct behavior:** `breakpoint` (by address, and by
place) refuses an address that is not a multiple of 4 with that same message, and
never writes it.

References:

- `src/docs/spec/tooling/10_debugger.md` *Breakpoints*: "**By address**: one
  breakpoint at that address …"; "**By place** (`<name>+<offset>` …): one
  breakpoint `<offset>` bytes into each bound function". *macOS backend →
  Breakpoints*: "`insert_breakpoint` saves the 4 bytes at the address and writes
  `brk #0`". The refusal table has the `dis` alignment row ("`dis` at an address
  not a multiple of 4") but no matching `breakpoint` row.
- Found during hardening batch B14 (tooling 07–10).

## Failing Reproduction

```
mfb init /tmp/dw && cd /tmp/dw
cat > src/main.mfb <<'EOF'
IMPORT io
FUNC count(n AS Integer) AS Integer
  MUT total AS Integer = 0
  FOR i = 1 TO n
    total = total + i
  NEXT
  RETURN total
END FUNC
FUNC main() AS Integer
  io::print(toString(count(4)))
  RETURN 0
END FUNC
EOF
mfb build -q --debug -O0
cp $(which mfb) /tmp/mfb-dbg && sh scripts/sign-debugger.sh /tmp/mfb-dbg
printf 'bp count+2\nlist breakpoint\nrun\nq\n' | /tmp/mfb-dbg dbg build/dw.out
```

- Observed (binary /tmp/hardening/mfb-head, HEAD 28a15a153, signed copy):
  ```
  DBG> bp count+2
  Set a breakpoint at 0x00000001043f4776 in count+0x2.
  DBG> list breakpoint
  Address            Symbol            Written name
  0x00000001043f4776 _mfb_fn_count+0x2 count+0x2
  DBG> run
  --[XC] T: 0 P: 0x00000001043f4774 F: count X: an illegal instruction
  ```
- Expected: `error: 0x00000001043f4776 is not the start of an instruction: they
  start every 4 bytes`, no breakpoint set, and `run` printing `10` and ending
  normally.

Contrast: `dis count+3` in the same session is refused with `… is not the start of
an instruction: they start every 4 bytes`. `bp count` and `bp count+4` work. The
same corruption follows from `bp 0x<odd address>`.

## Root Cause

`src/debugger/breakpoints.rs:Breakpoints::set` turns `Target::Address` and
`Target::Offset` into addresses, then calls `inferior.insert_breakpoint(address)`
with no alignment check. `src/debugger/macos.rs` `Task::insert_breakpoint` reads 4
bytes and patches `BREAKPOINT_INSTRUCTION` at any address it can write. The 4-byte
rule exists only in `src/debugger/disassemble.rs` (the `dis` window check).
`restart` re-places breakpoints by symbol plus offset, so an unaligned one is
written again into the new process.

## Goal

- `bp <addr|place>` whose address `% 4 != 0` is refused with the `dis` alignment
  text, and inserts nothing.
- `breakpoint r` of such an address still answers `there is no breakpoint at …`.

### Non-goals (must NOT change)

- Aligned breakpoints, pending library breakpoints, and the debugger's own
  park/entry/finish breakpoints. Those are always aligned.

## Blast Radius

- `Breakpoints::set` (address and place) is fixed by this bug.
- `Finish` return-instruction breakpoints are unaffected: their addresses come from
  decoded instruction starts.
- The `breakerror` park breakpoint is unaffected: it sits at a function symbol.
