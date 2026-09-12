# cli

`cli` defines a declarative schema for strict command-line options. You write a
`List OF cli::Option` once; it drives parsing, the typed accessors, and the
usage text, so the options a program documents and the options it accepts cannot
drift apart.

Build and test the package from the repository root:

```sh
mfb build packages/cli
mfb test packages/cli
packages/cli/smoke.sh target/release/mfb
packages/cli/check-doc-examples.sh target/release/mfb
```

Place the built artifact at the consumer-relative path `packages/cli.mfp` and
declare it as
`{ "name": "cli", "version": "=0.1.0", "source": "file:packages/cli.mfp" }`.

```mfb
IMPORT cli

LET options AS List OF cli::Option = [cli::Option[name := "port", alias := "p", required := TRUE, kind := cli::OptionKind.Integer], cli::Option[name := "verbose", alias := "v", required := FALSE, kind := cli::OptionKind.Flag], cli::Option[name := "enabled", alias := "b", required := FALSE, kind := cli::OptionKind.Bool]]
cli::parse(options)
LET port AS Integer = cli::getInteger("port", 3000)
LET verbose AS Boolean = cli::getFlag("verbose", FALSE)
LET enabled AS Boolean = cli::getBool("enabled", FALSE)
```

Call `parse` once, from the main thread, before starting any worker thread: it
publishes into package-level state that every accessor reads, and does not lock
it.

Render the full API — every accessor, its errors, and a runnable example — with:

```sh
mfb pkg doc packages/cli/cli.mfp --out cli-doc.html
```

## Accepted spellings

| Spelling | Meaning |
|---|---|
| `--name value`, `-a value` | The value is the next argument. |
| `--name=value` | The value is joined to the option. |
| `--flag`, `-f` | A `Flag`, which takes no value. |
| `--` | Ends option parsing; the rest are operands. |

`--name=value` is the only spelling that can pass a value beginning with `-`,
because a *separate* argument starting with a dash is read as a missing value
rather than as the value. Write `--offset=-3`, not `--offset -3`. A short option
never takes an inline value: `-p=3000` fails and says so.

Everything after a bare `--` is an operand, whatever it looks like, and is read
back in order with `cli::operands()` — that is how a file named `-v` survives a
strict parser.

## Values and defaults

Values stay raw until a typed getter reads them. Each getter returns its
`fallback` only when the option was **absent**; an option given an empty value
(`--title=` or `-t ""`) was supplied, and reads back as `""`. Boolean values are
exactly `true`, `false`, `1`, or `0`.

Reading an option the schema does not declare, or reading a `String` option with
`getInteger`, fails at the call — those are mistakes in the program, and they
fail whatever the user typed.

## Failures

Parsing is strict; there is no lenient mode.
`errorCode::ErrInvalidArgument` covers a malformed schema, an unknown switch, a
duplicate, a missing value, a flag given a value, a short option spelled with
`=`, a malformed boolean, and calling `parse` twice with different schemas.
A missing required option raises `errorCode::ErrNotFound`; a malformed integer
raises `errorCode::ErrInvalidFormat`, naming the option that carried it.

## Deliberate limits

No subcommands, no automatic `--help` or `--version`, no repeated (list-valued)
options, no clustered short flags (`-abc` is one unknown option), and no
environment-variable or config-file fallback. Declare `--help` as a `Flag` and
call `cli::showUsage(header, options, footer)` yourself.
