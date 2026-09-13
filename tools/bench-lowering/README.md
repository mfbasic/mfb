# tools/bench-lowering

Repeatable lowering/register-allocation benchmark (plan-78), the before/after
performance gate for middle-end work.

- **bench-lowering.sh** — builds the debug and release `mfb`, then times a cold
  `mfb build -q -ncode` of each probe under both, and `mfb test tests/acceptance`.
  Takes the tree's gate lock (it writes dumps beside the probes). No arguments.

      bash tools/bench-lowering/bench-lowering.sh

- **probes/** — the fixed projects it times: `trivial` (a one-line program) and
  `one-regex` (one constant `regex::match`, which inlines the whole regex engine).
