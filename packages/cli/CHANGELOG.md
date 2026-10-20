# Changelog

All notable changes to the `cli` package. The package follows
[Semantic Versioning](https://semver.org/): from 1.0.0 on, a breaking change to
the exported API or to documented parsing behavior requires a new major
version.

## 1.0.0

The first stable release. The API was redesigned from 0.1.0, and code written
against 0.1.0 needs updating.

### Added

- `cli::Command` describes a whole command line: program name, summary,
  version, options, operand description and count limits, `optionsFirst`, and
  footer.
- Automatic `-h, --help` and (when `version` is set) `-V, --version`.
  `cli::parse` prints them and exits with status 0, even when a required
  option is missing.
- An argument that is not valid UTF-8 is reported as a usage error, and an
  unusable program path falls back to the executable's file name.
- Usage errors are printed as `<program>: <message>` plus a
  `Try '<program> --help'` hint on standard error, and the program exits with
  status 2. `cli::usageError` reports a program's own checks the same way.
- `cli::tryParse(command, args)` parses any argument list without printing
  or exiting, and returns a `cli::Args` with a `ParseStatus` and a message.
- `OptionKind.Float`, plus `Option` fields `help`, `defaultValue`,
  `valueName`, `choices`, and `repeatable`.
- Accessors `getFloat`, `getStrings`, `getIntegers`, `getFloats`, `has`, and
  `count`. Also `helpText`, `versionText`, and `programName`.
- GNU-style syntax: operands anywhere on the command line, a lone `-` as an
  operand, grouped short flags (`-vq`, `-ofile`), `-n=value`, and negative
  numbers as values.

### Changed

- `cli::parse` takes a `cli::Command` and returns a `cli::Args`. It no longer
  stores package-level state, so it is safe from any thread and may be called
  more than once.
- Accessors take the `cli::Args` as their first argument, and defaults come
  from the option's `defaultValue` instead of a per-call `fallback`.
  `getFlag` takes no fallback.
- Every value is checked against its kind and `choices` while parsing, so an
  invalid value is reported before the program runs and accessors never fail
  on user input.
- `Bool` also accepts `yes`/`no`/`on`/`off`, and every spelling is accepted
  in any letter case.
- `OptionKind.String` is the first member, so an option constructed without
  a `kind` takes a string.
- Option names are limited to letters, digits, `-` and `_`, and an alias
  must be a single letter, so a negative number is never mistaken for an
  option. A `Bool` option cannot be `repeatable`.
- The help table uses the option's name as the value placeholder, and shows
  each option's description, possible values, default, and `[required]`.

### Removed

- `cli::showUsage`; print `cli::helpText(command)` instead.
- The `fallback` parameters of the accessors.
- Operands are no longer restricted to after `--`.
