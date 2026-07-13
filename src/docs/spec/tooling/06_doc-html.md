# Documentation HTML (mfb doc)

`mfb doc` and `mfb pkg doc` render a package's documentation to a single
self-contained HTML file. Both share one rendering core: a source
of declarations is normalized into a `DocPage`, and the renderer emits the page.[[src/doc.rs:render_html]]
The two commands differ only in where the `DocPage` comes from — parsed `.mfb`
source vs. a compiled `.mfp` package's `doc` section. This topic specifies the
`DocPage` assembly rules, the HTML structure, the embedded stylesheet, and the
two commands' resolution and exit codes.

The DOC source syntax that feeds this renderer is `./mfb spec language
documentation`; the byte encoding of the `.mfp` `doc` section is `./mfb spec
package doc-section`. This topic owns only the rendering model.

## Two Sources, One Page

```text
mfb doc      → build_source_doc_page → doc::from_source(&AstProject) ─┐
                                                                      ├─→ DocPage → render_html → HTML
mfb pkg doc  → write_package_doc     → doc::from_package(PackageDocs) ┘
```

The two builders produce the same `DocPage` shape; the differences are entirely
in the input and the public/internal partition (below).

[[src/doc.rs:from_source]] [[src/doc.rs:from_package]] [[src/cli/doc.rs:build_source_doc_page]] [[src/cli/pkg.rs:write_package_doc]]

| Aspect | `from_source` (source DOC blocks) | `from_package` (compiled `.mfp`) |
|---|---|---|
| Input | parsed `AstProject` | decoded `PackageDocs` |
| Declarations included | all (exported **and** non-exported) | exported only (what the `.mfp` `doc` section carries) |
| Signature source | resolved from the matching AST `Function`/type decl | stored `signature` string |
| Prose kind | `DocProseKind` carried on the AST node | reconstructed from the stored 1-byte kind code via `DocProseKind::from_code` |
| Overload selection | header param types match a specific overload | already resolved at compile time |
| Files marked `INTERNAL` | skipped entirely (`file.internal`) | not present (never emitted to `.mfp`) |

`from_source` walks `ast.files`, skipping any file whose `internal` flag is set,
and first indexes every `Function` (by name, collecting overloads) and every
`Type`/`Union`/`Enum` (by name) before walking the `Doc` items so each DOC block
can be matched to its declaration. A `Func`/`Sub` DOC block with explicit header
parameter types selects the overload whose normalized param types match;
otherwise the first overload of the right kind (sub vs. func) is used. A DOC
block whose declaration cannot be resolved is silently dropped.

[[src/doc.rs:source_decl_meta]]

## DocPage Shape

```text
DocPage
  package_name        : String        ; page <title>, sidebar header, <h1>
  subtitle            : String        ; first DESC paragraph of the package block
  intro               : Vec<Prose>    ; remaining package prose/callouts
  package_deprecated  : Option<String>; package-level DEPRECATED message
  public              : Vec<DocGroup> ; exported / non-INTERNAL declarations
  internal            : Vec<DocGroup> ; non-exported or INTERNAL declarations

DocGroup { title: String, decls: Vec<DocDecl> }
Prose    { kind: DocProseKind, text: String }
```

[[src/doc.rs:DocPage]] [[src/doc.rs:DocGroup]]

A `DocDecl` carries everything needed to render one declaration card:

```text
DocDecl
  anchor       : String           ; unique slug, used as section id and nav href
  kind_label   : &str             ; badge text: Function|Subroutine|Type|Union|Enum
  badge_class   : &str            ; badge CSS class: function|type|union|enum
  member_label  : Option<&str>    ; members-table heading: Fields|Variants|Members
  name          : String
  signature     : String
  desc          : Vec<Prose>
  args          : Vec<(name, desc)>   ; Parameters table
  props         : Vec<(name, desc)>   ; members table (Fields/Variants/Members)
  ret           : String              ; Returns prose
  errors        : Vec<(code, desc)>   ; Errors table
  example       : String
  deprecated    : Option<String>
```

[[src/doc.rs:DocDecl]]

### Kind-derived labels and the Types group

The declaration `kind` string drives four lookups:

| `kind` | `kind_label` | `badge_class` | `member_label` | group |
|---|---|---|---|---|
| `func` (default) | Function | function | — | its `GROUP`, else `Functions` |
| `sub` | Subroutine | function | — | its `GROUP`, else `Functions` |
| `type` | Type | type | Fields | `Types` |
| `union` | Union | union | Variants | `Types` |
| `enum` | Enum | enum | Members | `Types` |

Note `sub` shares the `function` badge class but a distinct label. Callables
group by their first `GROUP` line (falling back to `Functions` when absent);
all type-like kinds collapse into a single `Types` group **regardless** of any
`GROUP` line.

[[src/doc.rs:kind_label]] [[src/doc.rs:group_title]] [[src/doc.rs:member_label]]

## Grouping and the Public/Internal Partition

`assemble_groups` takes a flat, source-ordered list of `(DocDecl, group_title,
is_internal)` and bins each decl into either the `public` or `internal` list of
groups. Within each partition, groups appear in **first-appearance order** (the
order their first member is encountered) and decls keep source order within a
group. Two decls with the same group title are merged into one `DocGroup`.

[[src/doc.rs:assemble_groups]]

A declaration is **internal** when either:

- it is **not exported** (`from_source`: `visibility != Export`; `from_package`:
  the stored `internal` flag — only exported decls reach the `.mfp`, but the flag
  is preserved), **or**
- it carries an `INTERNAL` attribute on its DOC block (case-insensitive match in
  `from_source`).

[[src/doc.rs:from_source]]

In `from_package`, every decl in the `.mfp` `doc` section is already exported, so
the internal partition there comes solely from the stored `internal` flag.

### Subtitle split

The package-level prose is split by `split_subtitle`: if the **first** prose
block is a plain `DESC` paragraph, it is removed and becomes `subtitle`; all
remaining blocks become `intro`. If the first block is a callout (WARN/INFO/SEC),
`subtitle` is empty and every block stays in `intro`.

[[src/doc.rs:split_subtitle]]

## Anchor Slugging

`anchor` slugs a declaration name into an id used as both the `<section id>` and
the sidebar `href`:

1. Each character is lowercased if ASCII-alphanumeric, else replaced with `-`.
2. The slug is deduplicated against a per-page `HashSet`: a collision appends
   `-2`, `-3`, … until unique.

The dedup counter starts at `2`, so the first duplicate of slug `foo` becomes
`foo-2`. The `used` set is shared across the whole page (one set per `DocPage`
build), so a `Types` member and a function with the same slug still get distinct
anchors.

[[src/doc.rs:anchor]]

## Inline Markup

Inline text rendering is intentionally minimal: the **only** inline markup is the
backtick code span. `inline` walks the text and toggles on each backtick; text
inside a pair is wrapped in `<code>…</code>`, text outside is emitted as-is.
**All** text (inside and outside code spans) is HTML-escaped via `escape`
(`&`→`&amp;`, `<`→`&lt;`, `>`→`&gt;`, `"`→`&quot;`). There is no bold, italic,
link, or list markup. An unterminated backtick is emitted literally as a `` ` ``
followed by the escaped remainder.

[[src/doc.rs:inline]] [[src/doc.rs:escape]]

Signatures and examples are rendered with `escape` only (no backtick handling) —
they appear inside `<pre><code>` and are taken verbatim.

[[src/doc.rs:render_decl]]

## Callouts

Three prose kinds beyond plain `DESC` render as colored callout boxes. The
callout body is run through `inline` (so backtick spans work inside callouts).

| `DocProseKind` | DOC keyword | CSS class | icon |
|---|---|---|---|
| `Desc` | `DESC` | — (plain `<p>`) | — |
| `Warn` | `WARN` | `warning` | ⚠️ |
| `Info` | `INFO` | `info` | ℹ️ |
| `Sec` | `SEC` | `danger` | 🛡️ |

Deprecation messages also render as a `warning` callout: a package-level
`DEPRECATED` becomes a callout under the `<h1>`, and a per-decl deprecation
becomes a callout inside the decl card. An empty message falls back to fixed text
(`"This package is deprecated."` / `"This declaration is deprecated."`); a
non-empty message renders as `"Deprecated. <message>"`.

[[src/doc.rs:callout]] [[src/doc.rs:render_prose]] [[src/doc.rs:render_decl]]

## Page Structure

`render_html` emits one `<!DOCTYPE html>` document with an embedded `<style>` and
a two-column `.container` (sidebar + main).

```text
<html><head>
  <title>{name} — Documentation</title>
  <style>{STYLE}</style>            ; full embedded stylesheet, no external assets
</head><body>
  <div class="container">
    <nav class="sidebar">
      .sidebar-header (package name)
      Overview → "Introduction" (#intro)   ; only if subtitle or intro present
      one .nav-section per public DocGroup
      "Internal" divider + public-style sections   ; only if internal non-empty
    </nav>
    <main class="main">
      <h1>{name}</h1>
      <p class="subtitle">…</p>            ; only if subtitle present
      deprecation callout                   ; only if package_deprecated
      <section id="intro">…intro prose…</section>
      content groups (public)               ; <h2>{group}</h2> then one card each
      <h2>Internal — not part of the public API</h2> + internal content groups
    </main>
  </div>
</body></html>
```

[[src/doc.rs:render_html]]

Notable conditionals:

- The **Overview** nav entry and the `#intro` section render only when subtitle
  or intro is non-empty. When a subtitle exists but intro is empty, an empty
  `<section id="intro"></section>` is still emitted so the nav anchor resolves.
- When both `public` and `internal` are empty, the main column emits
  `<p>No documentation is available.</p>`.
- A declaration card (`render_decl`) emits, in order: header (`<h3><code>name`
  plus the kind badge), the signature block (omitted if empty), a deprecation
  callout (if any), the description prose, the Parameters table, the members
  table (Fields/Variants/Members, only for type kinds), the Returns paragraph
  (omitted if empty), the Errors table, and the Example block (omitted if empty).
- Tables (`render_table`) are skipped entirely when they have no rows. The Errors
  table renders its code column in a `.error-code` span; other name columns use
  `<code>`.

[[src/doc.rs:render_decl]] [[src/doc.rs:render_table]] [[src/doc.rs:render_sidebar_groups]]

## Embedded Stylesheet

The `STYLE` constant is a single minified CSS string inlined into the `<head>`;
there are no external stylesheet or script references, so a rendered page is fully
self-contained and viewable offline. It defines a light theme via `:root` custom
properties, a sticky scrollable sidebar (`--sidebar-width: 260px`), `.section`
cards, the four badge color variants (`function`/`type`/`union`/`enum`), the
three callout variants (`info`/`warning`/`danger`), the `.error-code` style, and
a single responsive breakpoint at `max-width: 900px` that stacks the sidebar
above the content. `html{scroll-behavior:smooth}` and `.section{scroll-margin-top}`
make anchor navigation smooth.

[[src/doc.rs:STYLE]]

## Empty Package Page

When `mfb pkg doc` targets a compiled package whose `doc` section is empty
(`PackageDocs::is_empty`), `render_empty_html` builds a `DocPage` with the package
name and otherwise empty fields and renders it — producing the standard chrome
with the `No documentation is available.` body. This path is unique to
`pkg doc`; `mfb doc` always has at least the parsed source to work from.

[[src/doc.rs:render_empty_html]] [[src/binary_repr/mod.rs:PackageDocs]]

## Command Resolution and Exit Codes

### `mfb doc [--out <file>] [location]`

Renders from source. `location` defaults to the current directory (`.`). If it is
a directory, the project manifest is validated and the project is parsed and
resolved, and the renderer runs over the whole resolved project; if it is a single
file, that file is parsed into a one-file project and its DOC blocks are validated.
`--out` defaults to `doc.html`. [[src/resolver/mod.rs:resolve_project]] [[src/resolver/mod.rs:validate_project_docs]]

[[src/cli/doc.rs:run_doc_command]] [[src/cli/doc.rs:build_source_doc_page]]

| Exit | Condition |
|---|---|
| `0` | page written and all DOC blocks valid |
| `1` | page written but DOC validation reported problems (diagnostics already on stderr), **or** parse/manifest/IO failure |
| `2` | bad flag usage (`--out` without a value, unknown `--flag`, more than one `<path>`) |

Because validation failures still write the HTML (the page is built from whatever
parsed), a CI gate that wants "docs are clean" must check for exit `1`, not the
presence of the output file.

[[src/cli/doc.rs:run_doc_command]]

### `mfb pkg doc <name-or-path> [--out <file>]`

Renders from a compiled `.mfp`. The target resolves as: a path ending in `.mfp`
or any existing file is used directly; otherwise `packages/<name>.mfp` is tried.
A missing package is an error. The `.mfp` header and `doc` section are read; an
empty doc section yields the empty page (still exit success). `--out` defaults to
`doc.html`.

[[src/cli/pkg.rs:run_pkg_doc]] [[src/cli/pkg.rs:write_package_doc]]

`pkg doc` reports through `PkgCommandError`, which the top-level `pkg` dispatch
maps to exit codes:

| Exit | Condition |
|---|---|
| `0` | page written (including the empty-doc-section case) |
| `1` | `PkgCommandError::Failed` — package not found, decode failure, or IO error |
| `2` | `PkgCommandError::Usage` — `--out` without a value, unknown `--flag`, missing `<name-or-path>`, or more than one target |

[[src/cli/pkg.rs:run_pkg_command]] [[src/cli/pkg.rs:run_pkg_doc]]

## See Also

* ./mfb spec language documentation — the DOC block source syntax that feeds this renderer
* ./mfb spec package doc-section — the byte encoding of the `.mfp` doc section consumed by `pkg doc`
* ./mfb spec tooling cli-reference — the full command/flag/exit-code surface for `doc` and `pkg doc`
* ./mfb spec architecture commands — where `doc`/`pkg doc` sit among the non-build commands
* ./mfb spec package container-format — the `.mfp` container that carries the doc section
