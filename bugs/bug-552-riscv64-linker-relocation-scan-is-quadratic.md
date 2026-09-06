# bug-552: the linux-riscv64 linker resolves each `lo12` relocation by scanning the whole relocation list, so linking is O(R²)

Last updated: 2026-09-05
Effort: small–medium (an index replaces a scan; the measurement harness already exists)
Severity: MEDIUM (performance; output is correct)
Class: Performance / scalability

Status: Open

## The finding

`paired_auipc_offset` (`src/os/linux/link/mod.rs`) resolves each
`riscv_pcrel_lo12` / `riscv_got_lo12` relocation by scanning the **entire**
relocation list backwards for the nearest preceding `*_hi20` naming the same
target. That is O(R) per relocation and therefore **O(R²) per link**.

The output is correct. This is a scalability defect, not a miscompile.

## Why it was unreachable until 2026-09-05

A function large enough to feel it was rejected earlier, at encode time, by the
`jal` ±1 MiB range error that bug-453 fixed (`e361fcc76`). Removing that ceiling
is what made this measurable — so this bug is a direct consequence of that fix
and did not exist as a reachable path before it.

## Measured

`mfb build --target linux-riscv64 -vv`, both libc flavors, one generated
single-function project per row (the harness bug-453 built for its own repro):

| function text | `relax rv64 branches` | `encoding image` | `linking executable` |
| --- | --- | --- | --- |
| 2.5 MiB | 0.50 s | 0.50 s | 0.55 s |
| 12.6 MiB (5×) | 2.67 s (5.3×) | 2.42 s (4.8×) | 11.9 s (**21.6×**) |

The relaxation pass and the encoder scale linearly (5.3× and 4.8× for a 5×
input); the linker does not (21.6×). A 50 MiB single function still links, but in
minutes rather than seconds.

## Best fix

Build the index once per link — `target → sorted hi20 offsets` — and binary-search
it, replacing the backward scan. The pairing rule itself does not change; only
how the nearest preceding `hi20` is found.

## What a fix must show

- The two rows above re-measured, with `linking executable` scaling like the
  other two passes rather than quadratically.
- Byte-identical output: the linked image for an existing riscv64 fixture must not
  change, since only the lookup strategy moves. The artifact gate cannot see this
  (`.ncodesum` is pre-link), so the check is a byte-compare of a linked
  executable before and after — see bug-453's note on the same limitation.

## Non-goals

- Do not change the `hi20`/`lo12` pairing semantics.
- Do not "fix" it by capping function size; bug-453 exists precisely so large
  functions link.

References: `src/os/linux/link/mod.rs:paired_auipc_offset`;
`bugs/completed/bug-453-*` §"Found while fixing this" (the measurement above and
the harness that produced it).

Found by the bug-453 fix, which recorded it rather than expanding scope — the
output is correct and the subsystem is a different one.
