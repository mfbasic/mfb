# MFB Documentation System Plan

Last updated: 2026-06-21

This document specifies the `DOC` block syntax for inline documentation,
compiler validation rules, the new documentation section in compiled `.mfp`
packages, and the `mfb doc` / `mfb pkg doc` CLI commands that render HTML
output.

It complements:

- `specifications/mfbasic.md`
- `specifications/package_format.md`
- `specifications/project.md`

---

## 1. Motivation

Two facts about MFB make documentation more important than in most languages:

- **Compiled packages ship without source.** A `.mfp` file contains binary
  representation, not source text. Without a doc section in the package,
  language-server hover and `mfb pkg doc` have nothing to show for an imported
  package.

- **The error channel is invisible in the type signature.** Every `FUNC` can
  fail, effects are inferred, and `FUNC f(x) AS Integer` says nothing about
  which error codes the function produces or when. The `DOC` block's `ERROR`
  lines are the only place that contract can be expressed.

The design goal is documentation that the compiler owns — validated against the
declaration it describes, persisted in the package binary, and renderable
without source.

---

## 2. DOC Block Syntax

### 2.1 Structure

A `DOC` block contains a required header line followed by zero or more content
lines, in any order except that `EXAMPLE` is a sub-block:

```
DOC
  <header>
  [DESC ...]
  [ARG  ...]
  [RET  ...]
  [ERROR ...]
  [EXAMPLE
    ...
  END EXAMPLE]
END DOC
```

A `DOC` block may appear in one of two placements:

- **Attached** — immediately before the declaration it documents, with no blank
  lines, statements, or comments between `END DOC` and the declaration. This is
  the conventional placement for documentation that lives alongside the code.

- **Standalone** — anywhere in any `.mfb` source file in the same package,
  including a dedicated `doc.mfb` file. The block is not adjacent to the
  declaration; the header line alone identifies the target.

Both forms are fully equivalent. A package may freely mix attached and
standalone blocks. A `doc.mfb` file that contains only `DOC` blocks and
`IMPORT` statements is a valid source file.

A declaration may have at most one `DOC` block across all source files in the
package. Two blocks that name the same declaration are a compile error
(`DOC_DUPLICATE`).

### 2.2 Header Line

The first line after `DOC` names the kind and identifier of the declaration
being documented. The header is the compiler's sole link between a `DOC` block
and its target — it is used regardless of whether the block is attached or
standalone.

The header keyword must be `FUNC`, `SUB`, `TYPE`, `UNION`, `ENUM`, or
`PACKAGE`. The identifier must name a declaration of that kind in the same
package. A name that does not resolve to any declaration in the package is a
compile error (`DOC_UNRESOLVED`). A name that resolves to a declaration of a
different kind is a compile error (`DOC_NAME_MISMATCH`).

```basic
DOC
  FUNC createTable
  DESC ...
END DOC
EXPORT FUNC createTable(RES db AS Db, name AS String, columns AS Map OF String TO String) AS Nothing
```

```basic
DOC
  SUB logItem
  DESC Write one item to the log output.
  ARG  x  The value to log.
END DOC
EXPORT SUB logItem(x AS Integer)
```

The two blocks above are attached. The following is standalone — it may appear
in any file in the package, including a `doc.mfb` dedicated to documentation:

```basic
DOC
  FUNC createTable
  DESC CREATE TABLE from a column-name -> column-type map.
  ARG  db      The open database connection.
  ARG  name    Name of the new table.
  ARG  columns Map of column names to SQLite type strings.
END DOC

DOC
  SUB logItem
  DESC Write one item to the log output.
  ARG  x  The value to log.
END DOC
```

For a package-level doc the header is the keyword `PACKAGE` alone, with no
name:

```basic
DOC
  PACKAGE
  DESC SQLite database binding for MFBASIC.
  DESC Provides connection management, parameterized queries, and schema helpers.
END DOC
```

A `PACKAGE` doc block may appear in any source file in the package, including
`doc.mfb`. At most one `PACKAGE` block is allowed per package; a second is a
compile error (`DOC_DUPLICATE_PACKAGE`).

### 2.3 DESC Lines

One or more `DESC` lines provide the description. Multiple `DESC` lines are
concatenated in order with a single space between them. Backtick spans
(`` `like this` ``) are treated as inline code in HTML output. No other inline
markup is recognized.

```basic
DOC
  FUNC createTable
  DESC CREATE TABLE from a column-name -> column-type map.
  DESC Each entry becomes a `name type` column definition. `FOR EACH` over a
  DESC map binds a `MapEntry`, so `entry.key` is the column name and
  DESC `entry.value` its SQLite type string.
END DOC
```

A `DOC` block may have zero `DESC` lines; the description is then empty.

### 2.4 ARG Lines

`ARG <name> <description>` documents one parameter. The name must match a
declared parameter in the following function or sub signature; an unrecognized
name is a compile error (`DOC_ARG_UNKNOWN`). Not all parameters need an `ARG`
line; undocumented parameters are omitted from output. Multiple `ARG` lines for
the same parameter name are a compile error (`DOC_ARG_DUPLICATE`). `ARG` is not
valid in `TYPE`, `UNION`, `ENUM`, or `PACKAGE` doc blocks; it is a compile
error if present (`DOC_ARG_INVALID_CONTEXT`).

```basic
  ARG  db      The open database connection to create the table in.
  ARG  name    Name of the new table.
  ARG  columns Map of column names to SQLite type strings such as `INTEGER` or `TEXT`.
```

### 2.5 RET Line

`RET <description>` documents the return value of a `FUNC` or `SUB`. At most
one `RET` line is allowed per `DOC` block; a second is a compile error
(`DOC_DUPLICATE_RET`). `RET` is valid but optional when the return type is
`Nothing` or the declaration is a `SUB`. `RET` is not valid in `TYPE`, `UNION`,
`ENUM`, or `PACKAGE` doc blocks (`DOC_RET_INVALID_CONTEXT`).

```basic
  RET  The number of rows affected by the operation.
```

For a function returning `Nothing` where the line adds no information, `RET`
may be omitted entirely.

### 2.6 ERROR Lines

`ERROR <code> <description>` documents one error this function may produce. The
code is an integer literal matching the runtime `Error.code` value. Multiple
`ERROR` lines are allowed and are recorded in source order. In v1 the compiler
does not validate that documented codes can actually be emitted by the function
body; they are stored and rendered as-is.

```basic
  ERROR 77050002 name is an empty string.
  ERROR 77050003 columns map is empty.
  ERROR 77050004 a column type string is empty.
```

`ERROR` is valid on `FUNC` and `SUB` doc blocks. It is not valid on `TYPE`,
`UNION`, `ENUM`, or `PACKAGE` doc blocks (`DOC_ERROR_INVALID_CONTEXT`).

### 2.7 EXAMPLE Block

An `EXAMPLE` / `END EXAMPLE` sub-block contains illustrative MFBASIC source.
At most one `EXAMPLE` block per `DOC` block in v1; a second is a compile error
(`DOC_DUPLICATE_EXAMPLE`). Example code is stored as raw source text and
rendered as a code block in HTML output. It is not compiled or validated in v1.
Future versions may compile examples as doctests.

```basic
DOC
  FUNC createTable
  DESC CREATE TABLE from a column-name -> column-type map.
  ARG  db      The open database connection.
  ARG  name    Name of the new table.
  ARG  columns Map of column names to SQLite type strings.
  ERROR 77050002 name is an empty string.
  ERROR 77050003 columns map is empty.
  EXAMPLE
    IMPORT sqlite

    RES db AS Db = sqlite::openOrCreate("app.db")
    LET columns AS Map OF String TO String = { "id" := "INTEGER", "name" := "TEXT" }
    sqlite::createTable(db, "users", columns)
  END EXAMPLE
END DOC
```

---

## 3. What May Have a DOC Block

| Declaration | Header keyword | DESC | ARG | RET | ERROR | EXAMPLE |
|-------------|----------------|------|-----|-----|-------|---------|
| `FUNC`      | `FUNC <name>`  | yes  | yes | yes | yes   | yes     |
| `SUB`       | `SUB <name>`   | yes  | yes | yes | yes   | yes     |
| `TYPE`      | `TYPE <name>`  | yes  | no  | no  | no    | yes     |
| `UNION`     | `UNION <name>` | yes  | no  | no  | no    | yes     |
| `ENUM`      | `ENUM <name>`  | yes  | no  | no  | no    | yes     |
| Package     | `PACKAGE`      | yes  | no  | no  | no    | no      |

A `DOC` block is allowed on any exported or non-exported declaration. Only
exported declarations have their doc data emitted into the `.mfp` doc section;
non-exported declarations are documented in source for maintainers but not
persisted into the compiled package.

---

## 4. Compile-Time Error Codes

| Code                         | Meaning                                                                          |
|------------------------------|----------------------------------------------------------------------------------|
| `DOC_UNRESOLVED`             | Header name does not resolve to any declaration in the package.                  |
| `DOC_NAME_MISMATCH`          | Header keyword (`FUNC`, `SUB`, etc.) does not match the kind of the named declaration. |
| `DOC_DUPLICATE`              | Two `DOC` blocks in the package name the same declaration.                       |
| `DOC_ARG_UNKNOWN`            | `ARG` name does not match any parameter in the target signature.                 |
| `DOC_ARG_DUPLICATE`          | Two `ARG` lines for the same parameter name.                                     |
| `DOC_ARG_INVALID_CONTEXT`    | `ARG` in a `TYPE`, `UNION`, `ENUM`, or `PACKAGE` doc block.                     |
| `DOC_RET_INVALID_CONTEXT`    | `RET` in a `TYPE`, `UNION`, `ENUM`, or `PACKAGE` doc block.                     |
| `DOC_DUPLICATE_RET`          | More than one `RET` line in a doc block.                                         |
| `DOC_ERROR_INVALID_CONTEXT`  | `ERROR` in a `TYPE`, `UNION`, `ENUM`, or `PACKAGE` doc block.                   |
| `DOC_DUPLICATE_EXAMPLE`      | More than one `EXAMPLE` block in a doc block.                                    |
| `DOC_DUPLICATE_PACKAGE`      | More than one `PACKAGE` doc block in the package.                                |

---

## 5. Package Doc Section in `.mfp`

The Binary Representation gains a new optional `doc` section. The compiler
emits this section for any package that has at least one exported `DOC` block.
A consumer that does not understand the `doc` section ignores it; doc data does
not affect execution.

### 5.1 Stored Per Exported Declaration

For each exported declaration that has a `DOC` block the section records:

- Kind: one of `func`, `sub`, `type`, `union`, `enum`
- Fully-qualified declaration name (package-prefixed)
- Description: the concatenated `DESC` text (UTF-8)
- Args: ordered list of `{ name, description }` pairs, in declaration order
- Return description: the `RET` text, or empty if absent
- Errors: ordered list of `{ code, description }` pairs, in source order
- Example: the raw example source text, or empty if absent

### 5.2 Package-Level Entry

If a `PACKAGE` doc block is present, the section also records:

- Description: the concatenated `DESC` text
- Package name (same as the header `name` field)

### 5.3 Format

Binary layout details are deferred to a future revision of
`specifications/package_format.md`. The section is length-prefixed and
self-describing so consumers can skip it entirely if the section tag is
unrecognized.

---

## 6. CLI Commands

### 6.1 `mfb doc <path>`

Generates HTML documentation from a source package directory or a single
source file. The compiler parses all `DOC` blocks found in the source,
validates them, and renders output.

```
mfb doc ./sqlite
mfb doc ./sqlite/db.mfb
mfb doc ./sqlite --out ./docs/sqlite.html
```

Default output filename: `doc.html` in the current working directory. The
`--out <file>` flag overrides the output path.

Exits non-zero and writes diagnostics to stderr if any `DOC` block fails
validation. Valid blocks are still rendered; invalid blocks are skipped with an
inline error note in the output.

### 6.2 `mfb pkg doc <name-or-path>`

Generates the same HTML documentation from a compiled `.mfp` package file. The
argument is either a package name resolved through the project lockfile, or a
direct path to a `.mfp` file.

```
mfb pkg doc sqlite
mfb pkg doc ./packages/sqlite.mfp
mfb pkg doc sqlite --out ./docs/sqlite.html
```

If the package was compiled without a `doc` section (no exported `DOC` blocks),
`mfb pkg doc` writes a minimal HTML page noting that no documentation is
available and exits with code `0`.

---

## 7. HTML Output Format

A single flat `.html` file. No JavaScript, no external stylesheets, no
multi-page navigation, no external resources. The file is self-contained and
readable offline.

### 7.1 Page Structure

```
<package name> — Documentation
  Package description (if any)

  [one section per documented exported declaration, in source order]
```

### 7.2 Declaration Section Structure

Each documented declaration renders as:

1. **Signature** — the full declaration signature as a code block, e.g.
   `EXPORT FUNC createTable(RES db AS Db, name AS String, columns AS Map OF String TO String) AS Nothing`
2. **Description** — the concatenated `DESC` text, with backtick spans rendered
   as `<code>`.
3. **Parameters table** — one row per documented `ARG`: name (as code), description.
   Omitted if no `ARG` lines.
4. **Returns** — the `RET` text. Omitted if absent or if the return type is
   `Nothing` and no `RET` line was written.
5. **Errors table** — one row per `ERROR` line: code (as code), description.
   Omitted if no `ERROR` lines.
6. **Example** — the `EXAMPLE` source as a code block. Omitted if no `EXAMPLE`
   block.

Undocumented exported declarations (no `DOC` block) are not listed in the
output. The HTML does not attempt to enumerate all exports — only the ones with
docs.

### 7.3 No Styling Requirement

The initial implementation produces structurally correct HTML using only
semantic elements (`<h1>`, `<h2>`, `<table>`, `<pre>`, `<code>`, `<p>`). A
minimal inline `<style>` block for basic readability (monospace code, table
borders) is acceptable but not required. No framework, no external CDN
references.

---

## 8. Implementation Sequence

1. **Parser** — add `DOC` / `END DOC` as a new top-level construct in the
   front-end. Parse all line types. Store each as a free-standing `DocBlock`
   AST node; do not assume proximity to a declaration.
2. **Resolver** — after all source files in the package are parsed, resolve each
   `DocBlock` header name to its declaration. Check for `DOC_UNRESOLVED`,
   `DOC_NAME_MISMATCH`, and `DOC_DUPLICATE`. Then validate `ARG` names against
   the resolved signature, duplicate checks, and context restrictions. Emit
   `DOC_*` diagnostics.
3. **IR / Binary Representation** — attach validated `DocBlock` data to each
   exported declaration in the IR. Add the `doc` section encoder to the Binary
   Representation writer.
4. **`mfb doc`** — implement the source-path HTML renderer as a new subcommand.
5. **`mfb pkg doc`** — implement the `.mfp` doc-section HTML renderer as a new
   subcommand. Reuse the HTML renderer from step 4.
6. **Language server** — use the in-memory `DocBlock` data to populate hover
   tooltips for locally defined symbols, and read the `.mfp` doc section for
   imported package symbols.
