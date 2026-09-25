# brogue

A port of Brogue CE's dungeon generator to MFBASIC, written to exercise the
language and its runtime against a known-correct reference.

The port covers the terrain half of Brogue's `digDungeon()`, everything
before `addMachines()`: rooms and corridors, loops, doors and walls, lakes
and their liquids, the non-machine autogenerators (with the dungeon features
they spawn), and diagonal clean-up. Machines, bridges, stairs, items and
creatures are not ported yet.

Brogue generates each dungeon level deterministically from a per-level seed,
using integer and fixed-point arithmetic only, so for a given seed the port
must reproduce the original's output exactly. Any difference is a bug in the
port or in MFBASIC.

## Layout

| Path | Contents |
|---|---|
| `src/main.mfb` | Entry point and the dump modes the checks compare |
| `src/rng.mfb` | Brogue's RNG (`Math.c`), ported bit for bit |
| `src/grid.mfb` | Grid helpers and blob generation (`Grid.c`) |
| `src/dijkstra.mfb` | `dijkstraScan` (`Dijkstra.c`) |
| `src/level.mfb` | The map being generated (`pmap`) and terrain queries |
| `src/rooms.mfb` | Room design, attachment and loops (`Architect.c`, `carveDungeon`/`addLoops`) |
| `src/terrain.mfb` | Walls, lakes, dungeon features, autogenerators, diagonals (`Architect.c`) |
| `src/tables.mfb` | Terrain tables, **generated** by `gen/gen_tables.py` from the oracle |
| `gen/` | The table generator |
| `check/` | C drivers built from the oracle's own source, and the scripts that diff them against the port |
| `oracle/` | Brogue CE's C source, the reference implementation |

## Building and checking

```console
$ mfb build examples/brogue
$ ./examples/brogue/build/brogue.out rng 12345 20
$ ./examples/brogue/build/brogue.out dig 12345 3
$ examples/brogue/check/check-rng.sh
$ examples/brogue/check/check-terrain.sh 8 1
```

`check-rng.sh` compiles `check/rng_dump.c` against the oracle's `Math.c`, runs
it and the port over a fixed seed list covering both of Brogue's seed paths
(below and above 2^32), and diffs every draw. It exits 1 on the first mismatch.

`check-terrain.sh [seedCount] [firstSeed]` compiles `check/terrain_dump.c`
against the whole oracle engine and, for each level seed at every depth 1-40,
diffs the port's `dig` output against it: the carved grid, the grid after
loops, and the full map (all four layers, cell flags and gas volume) after
walls, lakes, liquid fill, autogenerators and diagonal clean-up, each with
the RNG draw count at that point. A mismatch names the first stage that
differs.

`dig` takes a *level* seed, the seed Brogue derives for each depth from the
game seed; deriving it from a game seed needs `initializeRogue`, which is not
ported yet.

## Regenerating the tables

`src/tables.mfb` is generated. `gen/gen_tables.py` builds a small C program
against the oracle, prints every tile, dungeon feature, autogenerator and
dungeon profile from the compiled tables, and writes them out as MFBASIC, so
no value is transcribed by hand. `gen/gen_tables.py --check` exits 1 if the
file is stale.

## oracle/

- Upstream: https://github.com/tmewett/BrogueCE
- Pinned commit: `dedc315833c93826f632cd47fd87c0eb35541bcf` (2026-09-20)
- License: AGPL-3.0 (`oracle/LICENSE.txt`)
- Local changes: the upstream `.github/` CI directory is removed; the source is
  otherwise unmodified. Harness code lives in `check/`, not in `oracle/`.
