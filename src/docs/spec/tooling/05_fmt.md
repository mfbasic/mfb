# Formatter (mfb fmt)

`mfb fmt` rewrites MFBASIC source in place, normalizing only **block
indentation** and **keyword capitalization**. Everything else — intra-line
spacing, string contents, comments, blank lines, and `DOC`/`LINK` block bodies —
is preserved byte-for-byte. The transform is pure and deterministic: the same
input and indent width always produce the same output. [[src/fmt.rs:format_source]]

This topic owns the reimplementable normalization rules. The CLI surface that
drives them — argument parsing, file selection, exit codes — is summarized below
and specified in full at `./mfb spec tooling cli-reference` and
`./mfb spec architecture commands`.

## Why lexical, not AST-based

A formatter must preserve comments and blank lines, both of which the real lexer
discards. `mfb fmt` therefore re-tokenizes each physical line with a small
purpose-built scanner that keeps the original text verbatim and classifies tokens
only as far as indentation and continuation decisions require. It never builds an
AST and never parses expressions. As a consequence:

- Identifiers, package members (`pkg::name`), and field accesses (`value.field`)
  are left untouched even when they spell a keyword.
- A malformed or partially-written line is still re-emitted (its keywords cased,
  its text otherwise intact); the formatter does not reject input. [[src/fmt.rs:scan_line]]

## Top-level line model

`format_source` splits the source on `\n`, strips a trailing `\r` from each line
(`strip_cr`), and walks lines maintaining a **block stack**. For each line, in
order: [[src/fmt.rs:strip_cr]]

| Line kind | Handling |
|-----------|----------|
| Blank (trims to empty) | Emit an empty line; vertical spacing is preserved 1:1. |
| `DOC` start | Hand the whole `DOC … END DOC` block to `format_doc_block`. |
| `LINK "lib"` start | Hand the whole `LINK … END LINK` block to `format_link_block`. |
| Anything else | Gather a logical line (with continuations), case it, re-indent it. |

Indentation is computed as `level * indent_width` spaces (`indent_str`); there
are no tabs in output. [[src/fmt.rs:indent_str]]

### Trailing newline

The joined output always ends in exactly one `\n`: if the last emitted line does
not already end in a newline, one is appended. An empty input string returns an
empty string (no newline added). [[src/fmt.rs:format_source]]

## Per-line scanning and keyword casing

`scan_line` walks one physical line character by character, re-emitting it with
keywords uppercased and collecting a vector of **significant tokens** (`Sig`)
used later for structure decisions. The `Sig` variants are: [[src/fmt.rs:Sig]]

| `Sig` | Produced by |
|-------|-------------|
| `Kw(Keyword)` | A word recognized as a keyword (and casing was not suppressed). |
| `Underscore` | A lone `_` (line-continuation marker when last on the line). |
| `DoubleColon` | `::` (used to detect a `FUNC name AS pkg::func` re-export alias). |
| `LParen` | `(` (used to tell `FUNC(…)` type annotations / parameterized funcs from aliases). |
| `Other` | Strings, numbers, and any other punctuation or identifier. |

### Casing rules

A bare word is uppercased iff it is a recognized keyword **and** none
of the suppression rules below apply.[[src/lexer.rs:lookup_keyword]] The match is case-insensitive, so already
upper or mixed-case keywords are normalized. [[src/fmt.rs:scan_line]]

- **Suppression after `.` and `::`.** A `.` or `::` token sets a `suppress` flag
  that is consumed by the next word, which is then emitted verbatim. This keeps a
  field (`node.next`), a package member (`pkg::step`), and an aliased function
  (`AS pkg::close`) un-cased even when the trailing name spells a keyword.
- **Whitespace does not suppress.** Spaces/tabs are emitted verbatim and do
  *not* clear `suppress` or the type-position tracking — they pass through without
  resetting word context.

### `Nothing` (type) vs `NOTHING` (value)

`Nothing` names the unit *type* and is written CapitalCamelCase; `NOTHING` is the
unit *value* and is uppercased like any keyword. The two are disambiguated by
**type position**: the previous significant word, lowercased, being one of `as`,
`of`, or `to`. (`OF` is itself a keyword but the test is on the lowercased word
text, because the previous-word tracker keys on text, not on keyword identity.)
In a type position `Keyword::Nothing` is emitted as `Nothing`; everywhere else as
`NOTHING`. The previous-word tracker is reset to `None` by any non-word token
(string, number, punctuation) and updated by every word arm. [[src/fmt.rs:scan_line]]

```text
LET x = nothing                         ' value   → NOTHING
value AS Nothing                        ' type    → Nothing
LET t AS Thread OF Nothing TO Integer   ' type    → Nothing
```

### Comments: `REM` and `'`

| Form | Trigger | Effect |
|------|---------|--------|
| `'` | A `'` outside a string | The `'` and everything to end of line is copied verbatim (no casing inside). |
| `REM` | The word `REM` (any case) at a **statement start** | The `REM` keyword and the rest of the line are copied verbatim. |

A *statement start* is the beginning of a line or the position immediately after
a `:` statement separator. `REM` is only a comment introducer there; elsewhere it
is treated as an ordinary word. The `stmt_start` flag is set true at line start,
re-set true after a single `:`, and cleared by every other token. [[src/fmt.rs:scan_line]]

### Literals and punctuation

- **Strings** (`"…"`) are copied verbatim including escapes: after a `"`, a `\`
  escapes the next character (it and its successor are both copied) and an
  unescaped `"` ends the string. A string contributes one `Sig::Other`.
- **Numbers** are copied verbatim; an integer part optionally followed by `.` and
  more digits (a decimal point is only consumed when followed by a digit, so a
  trailing `.field` is not absorbed). Contributes one `Sig::Other`.
- **`:`** alone → `Sig::Other`, opens a new statement (`stmt_start = true`).
  **`::`** → `Sig::DoubleColon`, suppresses the next word. **`:=`** → `Sig::Other`.
- **`.`** → `Sig::Other` and suppresses the next word (field access).
- Any other character is copied as-is; `(` becomes `Sig::LParen`, everything else
  `Sig::Other`. [[src/fmt.rs:scan_line]]

## Line continuation

A logical line is a leading physical line plus every following physical line that
the previous one continues into. A physical line **continues** when its last
significant token is `Sig::Underscore` (a trailing `_`). Continuation lines are
gathered into the logical line and:

- The **first** physical line is trimmed and re-indented to the computed block
  level.
- **Continuation** lines keep their original leading whitespace (only trailing
  whitespace is stripped), so hand-aligned continuations are not disturbed; their
  keywords are still cased. [[src/fmt.rs:format_source]]

## Block-structure operation model

Each logical line's `Sig` vector is reduced to a list of structure **ops** by
`structural_ops` + `classify`, then applied to the block stack by `apply_ops`.
The op set is: [[src/fmt.rs:Op]]

| `Op` | Meaning | Stack effect | Printed at |
|------|---------|--------------|------------|
| `Open(Block)` | Opens a block (`FUNC`, `IF`, `FOR`, `MATCH`, …). | push | current depth (before push) |
| `Else` | `ELSE` / `ELSEIF`: stays inside the open `IF`. | none | one level *out* (`len-1`) |
| `Case` | `CASE`: closes any previous case arm, opens a new one. | pop a `Case` if on top, then push `Case` | depth after the optional pop |
| `End(Option<Keyword>)` | `END X`: closes a block; the keyword names which (for `END MATCH`). | pop (and a leading `Case` first for `END MATCH`) | depth after pop |
| `Pop` | `NEXT` / `WEND` / `LOOP`: closes the top loop block. | pop | depth after pop |

`Block` variants: `Func`, `Sub`, `Type`, `Union`, `Enum`, `If`, `For`, `While`,
`Do`, `Match`, `Trap`, `Case`. [[src/fmt.rs:Block]]

### How a line's indent is chosen

`apply_ops` starts the line at `base = stack.len()` (the surrounding depth). The
line is **dedented to the op's resulting level only when its first significant
token is itself structural** (`first_structural`, tracked by `structural_ops`):
the line then prints at the level produced by its *first* op. A line whose first
token is not structural (e.g. an ordinary statement, or `RES x = open(p)
TRAP(e)`) prints at `base` while still applying its ops to the stack — so an
inline opener raises indentation for the *following* lines without dedenting
itself. [[src/fmt.rs:apply_ops]] [[src/fmt.rs:structural_ops]]

### `classify` disambiguations

`classify` decides whether each keyword is structural, given the previous keyword
and position: [[src/fmt.rs:classify]]

- **`END X`.** `END` emits `End`; the block keyword *after* `END` (the `FUNC` in
  `END FUNC`) is suppressed (it names the closed block, not a new opener) because
  its `prev_kw` is `END`.
- **`EXIT`/`CONTINUE` targets.** A loop/routine keyword after `EXIT` or
  `CONTINUE` (`EXIT FOR`, `CONTINUE DO`) names a target and opens nothing.
- **`DO WHILE` / `LOOP WHILE`.** `WHILE` after `DO` or `LOOP` is a loop
  *condition*, not a `WHILE … WEND` opener, so `DO WHILE c` opens exactly one
  block.
- **`FUNC`/`SUB` may be mid-line.** They are *not* required to be the first token
  (visibility/`ISOLATED`/`EXPORT` modifiers may precede), so `EXPORT FUNC f()`
  still opens a block. They are excluded as openers when after `EXIT`, when a
  func-alias, or when a func-*type* (below).
- **`MATCH`/`CASE`.** `MATCH` opens a `Match`; each `CASE` closes the previous
  `Case` arm (if on top) and opens a new one; `END MATCH` pops a trailing `Case`
  then the `Match`.

### Single-line vs multi-line `IF`

A multi-line `IF` is recognized only when the first keyword is `IF` **and** the
line's last significant token is `THEN`. Only then does `IF` emit `Open(If)`. A
single-line `IF cond THEN stmt [ELSE stmt]` has tokens after `THEN`, so it opens
nothing and the following line is not indented. `ELSE`/`ELSEIF` emit `Else` only
when they are the line's first token (so a single-line `… ELSE …` does not
dedent). [[src/fmt.rs:structural_ops]] [[src/fmt.rs:classify]]

A single-line loop (`FOR i = 1 TO 3 : … : NEXT`) opens and closes on one line, so
its ops net to zero and mid-line position is harmless. Likewise a single-line
`WHILE FALSE : WEND` nested in a single-line `IF` stays balanced.

### Func-alias and `FUNC(`-type non-openers

Two `FUNC`/`SUB` shapes declare no body and open no block: [[src/fmt.rs:classify]]

- **Re-export alias** `[vis] FUNC name AS pkg::func` — detected by a `::` token
  (`DoubleColon`) and **no** `(` (`LParen`) anywhere on the line (`func_alias`).
  The grammar uses exactly this shape, so a real signature returning a
  `::`-qualified type cannot collide.
- **Function-type annotation** `FUNC(…) AS T` / `SUB(…)` — detected when the
  keyword is immediately followed by `(` (`func_type`). This is a type, not a
  declaration, so `… AS FUNC(Integer) AS Integer` does not open a second block.

## DOC blocks

A `DOC … END DOC` block is re-anchored as a unit by `format_doc_block`. The
opener is recognized (`is_doc_start`) as the word `DOC` (any case) followed only
by attribute words (alphabetic, space, or tab — e.g. `INTERNAL`) or nothing.
[[src/fmt.rs:is_doc_start]] Re-indentation: [[src/fmt.rs:format_doc_block]]

```text
<base>     DOC [attrs]          ; header: only the DOC keyword is cased (doc_header)
<base+1>     free-form body     ; trimmed, re-anchored; text & casing verbatim (prose)
<base+1>     EXAMPLE
<base+2>       example source    ; relative indent preserved (see below)
<base+1>     END EXAMPLE
<base>     END DOC
```

- `base` is the surrounding block depth. The body sits at `base+1`; an `EXAMPLE`
  keyword at `base+1` and its source at `base+2`.
- Body lines are **trimmed and re-anchored** but their internal spacing and casing
  are preserved — the body is free-form prose where a word like `if` is not a
  keyword.
- `EXAMPLE` source is **not** re-cased and is anchored with `flush_example`:
  each line is shifted to `level` while **preserving its indentation relative to
  the least-indented line** in the example, so the code's own nesting survives
  while the block as a whole is re-anchored. Blank lines emit empty.
  [[src/fmt.rs:flush_example]]
- `END EXAMPLE` / `END DOC` are matched as exactly the two words `END` + keyword
  (any case). An unterminated `DOC` (EOF before `END DOC`) still flushes any
  pending `EXAMPLE` source so nothing is lost. [[src/fmt.rs:format_doc_block]]

## LINK blocks

A native-binding `LINK "lib" AS alias` block has its own DSL with `FUNC`/`FREE`
nesting and contextual words (`SYMBOL`, `ABI`, `return`, `OUT`) the keyword
tracker does not model, so it is re-indented by `format_link_block` from that
nesting alone, with **all text and casing preserved** (so a contextual `return`
in an `ABI` line is never recased). [[src/fmt.rs:format_link_block]]

The opener is recognized (`is_link_start`) as the word `LINK` (any case) whose
remainder, after whitespace, starts with `"` — the trailing string literal
distinguishes the block from any ordinary use of the word. [[src/fmt.rs:is_link_start]]

Indentation is tracked by a local `depth` starting at `base+1`: a line whose
first word is `FUNC`, `SUB`, or `FREE` is an **opener** (printed at `depth`, then
`depth` increases); `END FUNC`/`END SUB`/`END FREE` is a **closer** (`depth`
decreases first, then printed); `END LINK` returns to `base` and closes the
block; blank lines emit empty; every other line prints at the current `depth`.
[[src/fmt.rs:format_link_block]]

## CLI: flags, selection, exit codes

`mfb fmt [--check] [--indent N] [location]`. [[src/cli/fmt.rs:run_fmt_command]]

| Flag | Default | Meaning |
|------|---------|---------|
| `--indent N` / `--indent=N` | `2` | Indent width in spaces. Must be a non-negative integer; a bad value errors with exit `2`. |
| `--check` | off | Write nothing; report files that are not already formatted. |
| `location` | `.` | A single `.mfb` file or a project directory. At most one. |

**File selection** (`format_path`): a `.mfb` file is formatted alone (a
non-`.mfb` file is an error); a directory is treated as a project — its
`project.json` is validated and `selected_source_paths` enumerates the `.mfb`
input set (see `./mfb spec tooling source-selection`). [[src/cli/fmt.rs:format_path]]

**Behavior and exit codes:** [[src/cli/fmt.rs:run_fmt_command]] [[src/cli/fmt.rs:format_path]]

| Mode | Per file | Process exit |
|------|----------|--------------|
| Normal | Rewrite in place only if the formatted text differs; print `Formatted <path>` per change, else a summary line. | `0` |
| `--check` | Print `Not formatted: <path>` per file that would change; write nothing. | `1` if any file differs, else `0` |
| Argument error | — | `2` (bad flag, bad `--indent`, or >1 location) |
| I/O / project error | — | `1` (with an `error:` message) |

In `--check` mode, when one or more files would change, the formatter emits the
general diagnostic **`FMT_CHECK_FAILED`** (code `2-200-0101`, severity Error,
"one or more source files are not formatted (mfb fmt --check)") before exiting
non-zero. [[src/rules/table.rs:FMT_CHECK_FAILED]] [[src/cli/fmt.rs:format_path]]

## See Also

* ./mfb spec tooling cli-reference — every command, flag, and exit code in one place
* ./mfb spec tooling source-selection — how `selected_source_paths` builds the `.mfb` input set
* ./mfb spec architecture commands — build modes and where `fmt` sits among the commands
* ./mfb spec diagnostics rule-codes — the `FMT_CHECK_FAILED` diagnostic and its code
* ./mfb spec tooling auditability — the keyword-casing convention the formatter enforces
