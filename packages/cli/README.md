# cli

`cli` defines a declarative schema for strict command-line options. Build it with
`mfb build packages/cli`, then reference `file:packages/cli/cli.mfp` from an
importing project. The parser accepts `--name value` and `-a value`; flags use
`--flag` or `-f` without a value. Call `parse` once before worker threads, then
read cached values with `getInteger`, `getString`, `getBool`, or `getFlag`.
