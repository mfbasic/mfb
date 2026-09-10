# cli

`cli` defines a declarative schema for strict command-line options. Build it with
`mfb build packages/cli`, then reference `file:packages/cli/cli.mfp` from an
importing project. The parser accepts `--name value` and `-a value`; flags use
`--flag` or `-f` without a value. Call `parse` once before worker threads, then
read cached values with `getInteger`, `getString`, `getBool`, or `getFlag`.

Place the built artifact at the consumer-relative path `packages/cli.mfp` and
declare it as `{ "name": "cli", "version": "=0.1.0", "source": "file:packages/cli.mfp" }`.

```mfb
IMPORT cli

LET options AS List OF cli::Option = [
  cli::Option[name := "port", alias := "p", required := TRUE, kind := cli::OptionKind.Integer],
  cli::Option[name := "verbose", alias := "v", required := FALSE, kind := cli::OptionKind.Flag],
  cli::Option[name := "enabled", alias := "b", required := FALSE, kind := cli::OptionKind.Bool]
]
cli::parse(options)
LET port AS Integer = cli::getInteger("port", 3000)
LET verbose AS Boolean = cli::getFlag("verbose", FALSE)
LET enabled AS Boolean = cli::getBool("enabled", FALSE)
```

Values remain raw until a typed getter reads them. Boolean values are exactly
`true`, `false`, `1`, or `0`; malformed input, unknown switches, duplicates,
and missing values raise `errorCode::ErrInvalidArgument`. A missing required
option raises `errorCode::ErrNotFound`. `showUsage(header, options, footer)`
renders the same schema the parser accepts.
