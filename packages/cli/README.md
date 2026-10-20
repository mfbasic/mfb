# cli

Declarative command-line parsing for MFBASIC. Describe what a program accepts
once, as a `cli::Command`. That one value drives strict parsing, typed
accessors, and the generated `--help` text, so the options a program documents
and the options it accepts cannot drift apart.

- GNU-style syntax: `--name value`, `--name=value`, `-n value`, `-nvalue`,
  grouped short flags (`-vq`), operands anywhere, and `--` to end options.
- Typed values (`String`, `Integer`, `Float`, `Bool`, `Flag`), each checked
  while the command line is parsed, not when the program first reads it.
- Defaults, required options, allowed `choices`, repeatable options, and
  counted flags (`-vvv`).
- Automatic `-h, --help` and `-V, --version`, with the conventional exit
  statuses: `0` after help or version, `2` after a usage error.
- No global state: parsing returns a value, so it is testable and safe to use
  from any thread.

## Quick start

```mfb
IMPORT cli
IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET command AS cli::Command = cli::Command[summary := "Serve a directory over HTTP.", version := "1.0.0", operands := "[dir]", maxOperands := 1, options := [cli::Option[name := "port", alias := "p", kind := cli::OptionKind.Integer, defaultValue := "8080", help := "Port to listen on"], cli::Option[name := "host", defaultValue := "127.0.0.1", help := "Address to bind"], cli::Option[name := "verbose", alias := "v", kind := cli::OptionKind.Flag, help := "Log every request"]]]
  LET args AS cli::Args = cli::parse(command)
  MUT dir AS String = "."
  IF len(cli::operands(args)) = 1 THEN dir = collections::get(cli::operands(args), 0)
  io::print("serving " & dir & " on " & cli::getString(args, "host") & ":" & toString(cli::getInteger(args, "port")))
  IF cli::getFlag(args, "verbose") THEN io::print("verbose logging on")
  RETURN 0
END FUNC
```

```console
$ serve --help
Serve a directory over HTTP.

Usage: serve [options] [dir]

Options:
  -p, --port <port>         Port to listen on [default: 8080]
      --host <host>         Address to bind [default: 127.0.0.1]
  -v, --verbose             Log every request
  -h, --help                Print help
  -V, --version             Print version

$ serve --prot 80
serve: unknown option '--prot'
Try 'serve --help' for more information.
$ echo $?
2
```

`cli::parse` returns only when the command line is valid. For `--help` and
`--version` it prints to standard output and exits with status 0. For a
command line the command does not accept, it prints the two lines above to
standard error and exits with status 2.

## Installing

Add the package to a project with `mfb pkg add <owner>#cli`, or point a
dependency at a local build:

```sh
mfb build packages/cli        # writes packages/cli/cli.mfp
```

```json
{ "name": "cli", "version": "=1.0.0", "source": "file:packages/cli.mfp" }
```

## Describing a command

Both records are constructed by field name. A field you leave out takes its
empty value (`""`, `0`, `FALSE`, `[]`), which always means "not used".

`cli::Option`:

| Field | Meaning |
|---|---|
| `name` | Long spelling without dashes: `"dry-run"` → `--dry-run`. Required. |
| `alias` | Short spelling, one letter: `"n"` → `-n`. |
| `kind` | `String` (the default), `Integer`, `Float`, `Bool`, or `Flag`. |
| `help` | One-line description for the help text. |
| `required` | The command line must supply it. |
| `defaultValue` | What the accessor returns when the option is absent, written as the user would type it. Checked against `kind`, and shown in the help text. |
| `valueName` | Placeholder in the help text: `--port <number>`. Defaults to `name`. |
| `choices` | The only values a `String` option accepts. |
| `repeatable` | The option may appear more than once (not on a `Bool`); read every value with `getStrings`/`getIntegers`/`getFloats`, or count a flag with `count`. |

`cli::Command`:

| Field | Meaning |
|---|---|
| `name` | Program name for help and errors. Defaults to the executable's file name, without `.exe` or `.out`. |
| `summary` | Text above the `Usage:` line. |
| `version` | Enables `--version`, which prints `<name> <version>`. |
| `options` | The options. |
| `operands` | How operands are written in the `Usage:` line, such as `"<file>..."`. Leave it empty and the command accepts no operands. |
| `minOperands`, `maxOperands` | Operand count limits. `maxOperands` `0` means no limit. |
| `optionsFirst` | Stop parsing options at the first operand (for subcommands). |
| `footer` | Text after the option table. |

A malformed command (a duplicate name, a default that isn't a number for an
`Integer` option, a `required` flag, ...) is a bug in the program. Every call
to `cli::parse` fails on it, so it shows up the first time the program runs.

## Reading values

| Accessor | Returns |
|---|---|
| `getString`, `getInteger`, `getFloat`, `getBool` | The supplied value, else `defaultValue`, else `""`/`0`/`0.0`/`FALSE`. |
| `getFlag` | Whether the flag was given. |
| `getStrings`, `getIntegers`, `getFloats` | Every value of a `repeatable` option, in order. |
| `has` | Whether the option appeared on the command line at all. |
| `count` | How many times it appeared (`-vvv` → 3). |
| `operands` | The operands, in order. |

Values were checked during parsing, so an accessor never fails on user input.
It fails only on a program mistake: reading an option the command doesn't
declare, using the accessor for another kind, or reading a result that didn't
parse.

## Parsing rules

| Command line | Meaning |
|---|---|
| `--name value`, `--name=value` | A long option and its value. |
| `-n value`, `-nvalue`, `-n=value` | A short option and its value. |
| `-abc` | The flags `-a -b -c`. The first value-taking option in a group takes the rest of the token, or the next argument. |
| `file.txt`, `-` | Operands, wherever they appear. |
| `--` | Ends option parsing; everything after it is an operand. |

- A separate value may not look like an option: `--title -v` is an error
  instead of silently taking `-v`. Negative numbers (`--offset -3`) and a
  lone `-` are accepted. Anything else beginning with `-` must be written
  inline: `--title=-v`.
- An empty value is a value: `--title=` reads back as `""`, and `has` is `TRUE`.
- Booleans accept `true`/`false`, `yes`/`no`, `on`/`off`, and `1`/`0`, in
  any letter case.
- Option names must match exactly. There are no abbreviations.
- A non-repeatable option given twice is an error.
- `--help` and `--version` take effect where they appear, even when a
  required option is missing. An error earlier on the command line is
  reported instead.
- Declaring an option with alias `h` (or `V`) takes that letter. The
  automatic option then has only its long spelling.

## Testing and custom handling

`cli::tryParse(command, args)` parses any list of strings, and never prints
or exits. Check `status`: `ParseStatus.Ok`, `Help`, `Version`, or `Invalid`
(with the reason in `message`).

```mfb
IMPORT cli
IMPORT io

FUNC main() AS Integer
  LET command AS cli::Command = cli::Command[options := [cli::Option[name := "count", alias := "n", kind := cli::OptionKind.Integer]]]
  LET args AS cli::Args = cli::tryParse(command, ["-n", "three"])
  IF args.status = cli::ParseStatus.Invalid THEN io::print(args.message)
  RETURN 0
END FUNC
```

prints `invalid value 'three' for '--count': expected an integer`.

`cli::helpText`, `cli::versionText`, and `cli::usageError(command, message)`
are the pieces `parse` uses. `usageError` prints a message in the standard
format and exits with status 2, for problems only the program can detect.

## Subcommands

There are no built-in subcommands, but `optionsFirst` makes them a few lines.
Parse the global options and stop at the verb, then hand the rest to the
verb's own command:

```mfb
IMPORT cli
IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET top AS cli::Command = cli::Command[name := "tool", operands := "<command> [args...]", minOperands := 1, optionsFirst := TRUE, options := [cli::Option[name := "verbose", alias := "v", kind := cli::OptionKind.Flag]]]
  LET args AS cli::Args = cli::parse(top)
  LET rest AS List OF String = cli::operands(args)
  LET verb AS String = collections::get(rest, 0)
  IF verb <> "greet" THEN cli::usageError(top, "unknown command '" & verb & "'")
  LET greet AS cli::Command = cli::Command[name := "tool greet", options := [cli::Option[name := "name", defaultValue := "world"]]]
  LET sub AS cli::Args = cli::tryParse(greet, collections::drop(rest, 1))
  IF sub.status = cli::ParseStatus.Help THEN
    io::print(cli::helpText(greet))
    RETURN 0
  END IF
  IF sub.status = cli::ParseStatus.Invalid THEN cli::usageError(greet, sub.message)
  io::print("hello, " & cli::getString(sub, "name"))
  RETURN 0
END FUNC
```

## Not included

Environment-variable and config-file fallbacks, abbreviated option names,
`--no-flag` negation (use a `Bool` option: `--cache=false`), and wrapping of
help text to the terminal width.

## Development

From the repository root:

```sh
mfb test packages/cli                                   # unit tests
packages/cli/smoke.sh target/release/mfb                # a real importer: stdout, stderr, exit status
packages/cli/check-doc-examples.sh target/release/mfb   # every DOC example builds and runs
mfb pkg doc packages/cli/cli.mfp --out cli-doc.html     # the full API reference
```

See [CHANGELOG.md](CHANGELOG.md) for release notes. Licensed under the MIT
license in the repository's [LICENSE](../../LICENSE).
