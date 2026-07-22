# 21. Documentation (`DOC` blocks)

A `DOC … END DOC` block attaches compiler-validated documentation to a
declaration. Because compiled `.mfp` packages ship without source, documentation
that the compiler owns — validated against the declaration it describes,
persisted in the package binary, and renderable without source — is the only way
hover and generated docs work for an imported package.

```basic
DOC [INTERNAL]
  <header>            ' FUNC|SUB|TYPE|UNION|ENUM|RESOURCE <name>, or PACKAGE
  [DESC ...]          ' description paragraph; a blank DESC starts a new one
  [INFO ...]          ' an informational callout (works like DESC)
  [WARN ...]          ' a warning callout
  [SEC  ...]          ' a security callout (shield)
  [DEPRECATED ...]    ' marks the declaration deprecated (optional message)
  [GROUP <name>]      ' FUNC/SUB only — group this callable in rendered docs
  [ARG  name desc]    ' FUNC/SUB only — must name a parameter
  [RET  desc]         ' FUNC/SUB only — at most one
  [ERROR code desc]   ' FUNC/SUB only — documented error codes, source order
  [PROP name desc]    ' TYPE/UNION/ENUM only — field/variant/member
                      ' (a RESOURCE is opaque, so it has no members)
  [EXAMPLE
    ...               ' raw MFBASIC source, rendered as a code block
  END EXAMPLE]
END DOC
```

- **Placement** is free: a block may sit immediately before its declaration or
  stand alone in any source file (including a dedicated `doc.mfb`). The header
  line — not proximity — is the sole link to the target. A package allows at most
  one `PACKAGE` block.
- **`DESC`/`INFO`/`WARN`/`SEC`** are prose lines: consecutive lines of one kind
  concatenate into a block, and a blank line of that kind ends it. They interleave
  in source order, so a callout can sit between two paragraphs. `DESC` renders as
  a paragraph; `INFO`/`WARN`/`SEC` render as informational, warning, and security
  callouts. Backtick spans (`` `like this` ``) render as inline code; no other
  markup is recognized.
- **`GROUP <name>`** (FUNC/SUB only, at most one) groups the callable under a
  named heading and sidebar section in rendered docs. Type-like declarations are
  grouped under a derived "Types" heading.
- **`RESOURCE <name>`** documents a `RESOURCE` declaration. An exported resource
  is importer-visible — it names the handle in every signature that takes or
  returns one — so it carries documentation like any other exported declaration.
  Its rendered signature includes the registered close op
  (`EXPORT RESOURCE Db CLOSE BY sqlite.close`), because that is the observable
  contract of the handle: it names what the automatic drop actually calls. A
  resource is an opaque handle with no fields, so `PROP` is rejected on it
  (`DOC_PROP_INVALID_CONTEXT`), and it groups under the derived "Types" heading.
- **Overloads**: a header may carry a parenthesized parameter-type list to pick a
  specific overload, e.g. `FUNC query(Db, String, List OF String)` —
  whitespace is normalized, and `RES`, any `STATE T` clause, and parameter names
  are all omitted: a `RES db AS Db STATE Cursor` parameter is written `Db`. Each overload
  may then carry its own `DOC` block; a bare `FUNC name` (no parens) documents the
  function family. Two blocks naming the same overload are a `DOC_DUPLICATE`.
- **`INTERNAL`** (a flag on the `DOC` line) marks an exported declaration as not
  part of the supported public API — still callable, but rendered in the Internal
  section with an `internal` flag in the package. A non-exported declaration is
  automatically internal. `DEPRECATED` is orthogonal: a deprecated declaration
  stays in its section but renders a deprecation banner.
- **Persistence**: the compiler emits a `doc` section into the `.mfp` for every
  *exported* declaration that has a `DOC` block (and for the `PACKAGE` block).
  Non-exported declarations are documented in source for maintainers but never
  persisted. The section is optional and self-describing; a consumer that does
  not understand it skips it, and it does not affect execution or the ABI.

Validation runs in the resolver and rejects malformed blocks with `DOC_*`
diagnostics: unresolved or mismatched headers, header parameter types matching no
overload, duplicate blocks, unknown or duplicate `ARG`/`PROP` names, context
violations (e.g. `ARG`/`GROUP` on a `TYPE`, `PROP` on a `FUNC`), duplicate
`RET`/`EXAMPLE`/`DEPRECATED`/`GROUP`, and unknown or duplicate attributes.

The toolchain renders documentation to a single self-contained HTML file:

```text
mfb doc <path> [--out file]            ' from a source directory or .mfb file
mfb pkg doc <name-or-path> [--out file] ' from a compiled .mfp doc section
```

`mfb doc` renders both public and internal sections (including non-exported,
implicitly-internal declarations). `mfb pkg doc` renders only what the package
persisted. A package compiled without any exported `DOC` block yields a minimal
"no documentation available" page and exits zero.

## See Also

* ./mfb spec package doc-section — how validated `DOC` blocks are persisted in the `.mfp` binary
* ./mfb spec tooling doc-html — rendering `DOC` blocks into generated HTML docs
* ./mfb spec architecture frontend — where `DOC` validation runs, before monomorphization
