# DOC Section

The optional `DOC` section (id `17`) carries the package's documentation surface.
It is self-contained: all strings are stored inline as
`u32`-length-prefixed UTF-8, independent of `STRING_POOL`. It does not contribute
to the ABI hash. The compiler emits it only when the package has at least one
exported `DOC` block or a `PACKAGE` doc block.

A `Prose` list is a `u32` count followed by that many `(u8 kind, str text)`
blocks, where `kind` is `0`=paragraph, `1`=warning, `2`=info, `3`=security. The
blocks render in order, interleaving paragraphs and callouts.

```text
u8                         hasPackage (0 or 1)
if hasPackage:
  str                      packageName
  Prose                    description (paragraphs + callouts)
  u8 + (str if 1)          deprecated flag, then optional message
u32                        declCount
declCount * DocEntry

DocEntry:
  u16                      kind (0=func, 1=sub, 2=type, 3=union, 4=enum)
  str                      name
  str                      signature (rendered source-form declaration line)
  str                      group ("" if none; FUNC/SUB only)
  Prose                    description (paragraphs + callouts)
  u32 + (str,str)*         args  (name, description)
  u32 + (str,str)*         props (name, description)
  str                      return description ("" if none)
  u32 + (str,str)*         errors (code, description)
  str                      example source ("" if none)
  u8                       internal (1 = exported-but-not-public)
  u8 + (str if 1)          deprecated flag, then optional message
```

`str` is a `u32` byte length followed by that many UTF-8 bytes. A consumer that
does not recognize section id `17` skips it; doc data never affects execution. [[src/binary_repr/reader.rs:read_doc_table]]

## See Also

* ./mfb spec language documentation — `DOC` blocks in source
* ./mfb spec package compact-summary — the full section table
