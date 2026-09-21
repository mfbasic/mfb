# brogue

A port of Brogue CE's dungeon generator to MFBASIC, written to exercise the
language and its runtime against a known-correct reference.

## oracle/

`oracle/` is an unmodified clone of Brogue CE, the reference implementation the
MFBASIC port is checked against.

- Upstream: https://github.com/tmewett/BrogueCE
- Pinned commit: `dedc315833c93826f632cd47fd87c0eb35541bcf` (2026-09-20)
- License: AGPL-3.0 (`oracle/LICENSE.txt`)

Brogue generates each dungeon level deterministically from a per-level seed,
using integer and fixed-point arithmetic only, so for a given seed and depth
the port must reproduce the oracle's terrain cell for cell.
