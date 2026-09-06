# bug-552: the linux-riscv64 linker resolves each `lo12` relocation by scanning the whole relocation list, so linking is O(R²)

Last updated: 2026-09-06
Effort: small–medium (an index replaces a scan; the measurement harness already exists)
Severity: MEDIUM (performance; output is correct)
Class: Performance / scalability

Status: FIXED

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

## Reproduced and fixed (2026-09-06)

The 2.5 MiB / 12.6 MiB rows above were not reproduced verbatim — the harness they
came from is not committed. A fresh generator (a single `FUNC` of N blocks of
`{acc arithmetic; s = strings::left("lit<k>_pad…", 3); IF …}`, so each block
contributes a data reference and therefore one `pcrel_hi20`/`pcrel_lo12` pair)
has a much higher relocation density per source byte, and reproduces the defect
at a far smaller input. `mfb build --target linux-riscv64 -vv`, macOS host, both
libc flavors summed, as `-vv` reports them:

| input | `relax rv64 branches` | `encoding image` | `linking executable` |
| --- | --- | --- | --- |
| 2 000 blocks (258 KiB) | 0.42 s | 1.00 s | **5.05 s** |
| 10 000 blocks (1.29 MiB, 5x) | 1.81 s (4.3x) | 4.48 s (4.5x) | **306.78 s (60.7x)** |

The direction the report gives is confirmed and then some: relax and encode are
linear, the linker is not.

After the fix, same binary paths, same projects:

| input | `relax rv64 branches` | `encoding image` | `linking executable` |
| --- | --- | --- | --- |
| 2 000 blocks | 0.42 s | 1.00 s | 0.47 s |
| 10 000 blocks (5x) | 1.81 s (4.3x) | 4.48 s (4.5x) | **3.05 s (6.5x)** |

`linking executable` now scales like its two neighbours (6.5x vs 4.3x and 4.5x
for a 5x input) instead of 60.7x, and the 5x row's link is **100.7x faster** in
absolute terms (306.78 s -> 3.05 s).

## The fix

`HiRelocIndex` (`src/os/linux/link/mod.rs`), built once per link in
`patch_relocations`: `(hi kind, target) -> ascending offsets`, binary-searched
with `partition_point` for the greatest offset strictly below the `lo12`'s. That
is exactly the predicate the backward scan computed
(`max { offset | kind == hi_kind && target == lo.target && offset < lo.offset }`),
so the pairing rule is untouched. Only `*_hi20` kinds are indexed. The map never
decides emission order, so it cannot make codegen non-deterministic.

The offsets are sorted rather than assumed sorted: the encoder emits them in
offset order today and the old `max` did not care either way, but a binary search
over an unsorted list silently returns the WRONG `auipc` instead of failing.
`the_pairing_index_does_not_assume_relocations_arrive_sorted` is that pin.

### Instrument — what proves the output did not move

The artifact gate **cannot see this**: `.ncodesum` is pre-link (see
`.ai/testing-gates.md` "Linker-stage changes are invisible to the artifact-gate").
It was run anyway and reports 1393 tests / 1937 goldens / **0 diffs**, which only
proves no codegen path was disturbed.

The correctness instrument is a **byte-compare of linked riscv64 executables**
before and after, since only the lookup strategy moves. Built with the
`8f0ebfeb8` binary, then with the fixed one, and `cmp`'d:

* `examples/hello_world`, `ai_chat`, `life`, `snake`, `network-client`, `hangman`
  — both libc flavors each, 12 images;
* the generated reproduction project, both flavors, 2 images.

**14 of 14 byte-identical.** The set covers both pairing kinds — `riscv_pcrel_lo12`
(internal data) throughout, and `riscv_got_lo12` (imported data globals) via the
libc-importing examples.

Unit pins (`src/os/linux/link/tests.rs`): the three pre-existing pairing tests are
unchanged apart from how they obtain the answer, plus
`the_pairing_index_is_keyed_by_kind_as_well_as_target` (a nearer `got_hi20` for
the same target must not be paired with a `pcrel_lo12` — the mistake a
target-only index invites), `the_pairing_index_does_not_assume_relocations_arrive_sorted`,
and `a_lo12_with_no_preceding_hi20_keeps_its_diagnostic`.

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
