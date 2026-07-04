# format

Render a `DateTime` as text with the pattern mini-language.

## Synopsis

```
datetime::format(dt AS DateTime, pattern AS String) AS String
```

## Package

datetime

## Imports

```
IMPORT datetime
```

`datetime` is a built-in package, so no manifest dependency is required.
[[src/builtins/datetime.rs:augmented_project]]

## Description

`datetime::format` renders the fields of `dt` as text by walking `pattern` from
left to right and emitting, for each position, either a literal character or the
value selected by a formatting token. The result is a freshly built `String`;
`dt` is read only and is not modified. [[src/builtins/datetime_package.mfb:__datetime_format]]

A token is a run of one or more of the same letter. The run length selects the
width or style of the field. Any character that is not a recognized formatting
letter is copied to the output verbatim, so separators such as spaces, dashes,
colons, and slashes appear literally. To emit a letter that would otherwise be
read as a token, wrap it in single quotes (`'T'` produces a literal `T`); to emit
a literal apostrophe, write two single quotes (`''`). [[src/builtins/datetime_package.mfb:__datetime_format]]

The recognized tokens are:

- `yyyy` / `yy` — year, zero-padded to the run length / last 2 digits
- `M` / `MM` — month number, minimal (1-12) / 2-digit
- `MMM` / `MMMM` — month name, short / full (English)
- `d` / `dd` — day of month, minimal / 2-digit
- `H` / `HH` — hour on a 24-hour clock (0-23), minimal / 2-digit
- `h` / `hh` — hour on a 12-hour clock (1-12), minimal / 2-digit
- `m` / `mm` — minute, minimal / 2-digit
- `s` / `ss` — second, minimal / 2-digit
- `fff` .. `fffffffff` — fractional second, fixed width (3 / 6 / 9 = ms / us / ns)
- `a` — AM/PM marker (AM before noon, PM at or after noon)
- `EEE` / `EEEE` — weekday name, short / full (English)
- `Z` — offset: the letter `Z` when the offset is zero, else `+/-HH:MM`
- `ZZ` — offset, always `+/-HH:MM` (`Z` is never substituted)
- `ZZZ` — offset, `+/-HHMM` with no colon

The year token zero-pads to the run length: `yyyy` pads to at least 4 digits,
while `yy` emits the last two digits of the year. The fractional-second token
takes the `nanos` of `dt.time`, renders them as 9 digits, and keeps the leading
run-length digits, so `fff` yields milliseconds, `ffffff` microseconds, and
`fffffffff` nanoseconds. Month, weekday, and AM/PM names are English. The offset
tokens read `dt.offset`, the resolved UTC offset carried by `dt`. [[src/builtins/datetime_package.mfb:__datetime_formatToken]]

Inside single quotes every character, including formatting letters, is copied
literally until the closing quote; an opening quote with no matching close runs
to the end of `pattern`. `datetime::format` is pure: it reads no host state and
has no side effects. [[src/builtins/datetime_package.mfb:__datetime_format]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `dt` | `DateTime` | The moment to render. Its date fields (`year`, `month`, `day`), time fields (`hour`, `minute`, `second`, `nanos`), and resolved UTC `offset` supply the values for the pattern tokens. [[src/builtins/datetime.rs:FORMAT]] |
| `pattern` | `String` | The format string: a mix of literal characters and token runs drawn from the table above, with single quotes escaping literal text. An empty pattern produces an empty result. [[src/builtins/datetime_package.mfb:__datetime_format]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The rendered text, with each token replaced by the corresponding field of `dt` and every other character copied through unchanged. The empty pattern returns the empty string. [[src/builtins/datetime.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `pattern` contains a run of letters that is not one of the recognized formatting tokens. [[src/builtins/datetime_package.mfb:__datetime_formatToken]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Render a `DateTime` with a full date, time, and offset:

```
IMPORT datetime

LET dt AS DateTime = datetime::toUtc(datetime::now())
LET text AS String = datetime::format(dt, "EEEE yyyy-MM-dd HH:mm:ss Z")
```

Use single quotes to include literal letters in the output:

```
IMPORT datetime

LET dt AS DateTime = datetime::toUtc(datetime::now())
LET text AS String = datetime::format(dt, "yyyy-MM-dd'T'HH:mm:ss")
```

## See also

- `mfb man datetime parse`
- `mfb man datetime toIso`
- `mfb man datetime formatDuration`
