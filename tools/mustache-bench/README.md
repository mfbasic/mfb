# mustache-bench — render time against the size of the context

Measures `mustache::renderWith` over four template shapes at several context
sizes, and checks that each step up in size costs at most 4.4× the step before it.
**Offline tooling only**: not in CI, not a gate. Its numbers are for a human to
read and for the CHANGELOG's known-limitation note to cite.

## Run

```
tools/mustache-bench/run.sh target/release/mfb
SIZES="4000 16000 64000 256000" tools/mustache-bench/run.sh target/release/mfb
```

Needs `target/release/mfb` (or a path passed as the first argument) and the
package at `packages/mustache`.

## What it measures

- **Shapes**: `flat` (`{{#items}}<li>{{name}}</li>{{/items}}`), `wrapped` (the same
  list one object down, under `data`), `partial` (`{{>row}}` inside the loop, so the
  partial is parsed on every item), and `outer` (a name from the root resolved
  inside the loop, so every item looks past its own frame).
- **Timing**: the probe builds its context, then renders five times in process,
  timing each `renderWith` call alone with `datetime::monotonicNanos`. The reported
  value is the median. Parsing the input and building the JSON are excluded.
- **Linearity**: for each shape, each step to a size four times larger must cost at
  most 4.4× the previous step. A quadratic render costs about 16× per step.

## Reading the result

The last measured run (2026-10-20, `SIZES="4000 16000 64000 256000"`) failed every
shape at the 64 000 → 256 000 step. The step ratios were 16.7× (`flat`), 15.3×
(`wrapped`), 11.7× (`partial`) and 6.5× (`outer`). The default sizes fail too: a
later default run (`4000 16000 64000`, same day) put the 4 000 → 16 000 step at
4.7× to 5.7× for every shape, and `outer`'s 16 000 → 64 000 step at 5.6×. The cause is in the language runtime, which the package's CHANGELOG.md
describes: the arena allocator's large-block flush sweeps every parked small block
on each flush. A build with the flush disabled, measured in a scratch build, took
the same flatten from 2.0 s to 0.36 s at 256 000 items. Until that runtime change
lands, a failing step here is the known limitation, not a regression in the
package's flattening.
