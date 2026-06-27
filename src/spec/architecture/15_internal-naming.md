# Internal Symbol Naming

The built-in packages `json`, `regex`, and `collections` are injected into every
build as MFBASIC **source** (`src/builtins/*.mfb`), so their private helpers live
in the same name space as user code. To keep those helpers from colliding with —
or being impersonated by — symbols the user authors, the compiler rewrites them to
an **unforgeable** sigil form during lexing and carries that form unchanged
through the AST, IR, and native code generation. [[src/internal_name.rs:internalize]]

## The sigil

`INTERNAL_SIGIL` is `'#'`. It is the marker that makes a name compiler-internal.
The character is deliberately one the lexer **never accepts inside a user
identifier**: in normal mode `#` falls through to the catch-all arm and is
reported as `MFB_LEX_UNEXPECTED_CHARACTER`, so no user program can produce a token
containing it. A name carrying `#` therefore could only have been minted by the
compiler. [[src/internal_name.rs:INTERNAL_SIGIL]] [[src/lexer.rs:262]]

```text
INTERNAL_SIGIL = '#'
```

### Why not `$`

`$` is *not* used for internal naming because it is already overloaded by the rest
of the pipeline: synthesized lambda names use it (`$lambda0`) and the
monomorphizer's concrete-symbol mangling uses it as the type-token delimiter
(`name$<san(T1)>$<san(T2)>`). Reusing `$` would conflate three unrelated naming
domains. `#` is free and untypeable, so it is reserved exclusively for this
purpose. [[src/internal_name.rs:INTERNAL_SIGIL]] [[src/ir.rs:3051]]

See `./mfb spec architecture monomorphization` for the `$` mangling scheme and
`./mfb spec language lexical-structure` for the lexer's reserved-character set.

## Convention vs. guarantee

The injected packages name their private helpers with a `__pkg_name` convention
(e.g. `__json_parse`, `__collections_sort`, `__regex_match`). On its own this is
only **probabilistic** protection — nothing stops a user who imports the package
from declaring a colliding `__pkg_name`. The sigil rewrite is what upgrades the
convention into a hard guarantee. [[src/internal_name.rs:internalize]]

## Internalize: lexer rewrite

`lex_with(path, source, internal)` takes an `internal` flag; only the injected
built-in files are lexed with it set (`AstFile::internal`). When the flag is set,
each identifier whose value begins with `__` is passed through `internalize`,
which strips the `__` prefix and prepends the sigil. The rewrite happens after
keyword classification (keywords never carry a `__` prefix) and `DOC`/`REM`
handling, so it only ever affects names. [[src/lexer.rs:lex_with]] [[src/lexer.rs:410]]

```text
internalize("__json_parse")        -> "#json_parse"
internalize("__collections_sort")  -> "#collections_sort"
internalize("Json")                -> "Json"      (no `__` prefix: untouched)
internalize("parse")               -> "parse"     (untouched)
internalize("_json")               -> "_json"     (single underscore: untouched)
```

Only a leading `__` triggers the rewrite. Public package types such as `Json` and
public package functions such as `parse` have no `__` prefix and pass through
unchanged, so the package's exported surface is unaffected. A user file (lexed
with `internal = false`) is never rewritten, and the user cannot type `#`, so the
sigil name is unreachable from user code. [[src/internal_name.rs:internalize]]

## Lifetime through the pipeline

The sigil name is a plain string from the lexer onward; it survives unchanged
through the AST and into the IR, where it guarantees no collision with any user
symbol. The IR construction re-applies `internalize` when synthesizing references
to internal definitions. [[src/ir.rs:3015]]

| Stage          | Form                | Notes                                            |
|----------------|---------------------|--------------------------------------------------|
| Lexer (normal) | `#` rejected        | `MFB_LEX_UNEXPECTED_CHARACTER`                   |
| Lexer (internal)| `__name` -> `#name`| `internalize`, leading `__` only                 |
| AST / IR       | `#name`             | opaque string, no user symbol can equal it       |
| Codegen        | `_mfb_ifn_<name>`   | sigil stripped, reserved native namespace        |
| Diagnostics    | `__name`            | `display_name` maps `#` back to `__`             |

## Strip sigil: codegen mapping

At native code generation the sigil is removed and the remainder is routed into a
reserved symbol namespace. `function_symbol` calls `strip_sigil`: a name that
carries the sigil becomes `_mfb_ifn_<fragment>`, while every ordinary user/package
function becomes `_mfb_fn_<fragment>`. Because the two namespaces are disjoint and
the user can never mint a sigil name, an internal function can never collide with
a user function at link time. `strip_sigil` returns `None` for a plain
`__`-prefixed string (as a user would type), so a user-authored `__name` is *not*
treated as internal. [[src/internal_name.rs:strip_sigil]] [[src/target/shared/nir.rs:function_symbol]]

```text
strip_sigil("#json_parse")  -> Some("json_parse")
strip_sigil("__json_parse") -> None              (no sigil: user-typeable)

function_symbol("#json_parse") -> "_mfb_ifn_json_parse"
function_symbol("parse")       -> "_mfb_fn_parse"
```

The `<fragment>` is produced by `symbol_fragment`, which maps every character
outside `[A-Za-z0-9_]` to `_`. See `./mfb spec architecture native` for the native
IR symbol model and `./mfb spec linker symbols-and-relocations` for the
`_mfb_ifn_` / `_mfb_fn_` namespace catalog. [[src/target/shared/nir.rs:symbol_fragment]]

## Display name: diagnostics

The untypeable sigil must never leak into user-facing messages. `display_name`
maps a sigil name back to its readable `__` form for diagnostics (and returns
non-internal names unchanged). The monomorphizer uses it when reporting errors
against internal generic implementations such as `collections::sort`, so the user
sees `__collections_sort` rather than `#collections_sort`.
[[src/internal_name.rs:display_name]] [[src/monomorph.rs:433]]

```text
display_name("#collections_sort") -> "__collections_sort"
display_name("parse")             -> "parse"
```

## See Also

- `./mfb spec language lexical-structure` — `#` as a reserved, never-accepted character
- `./mfb spec architecture monomorphization` — `$`-delimited concrete-symbol mangling
- `./mfb spec architecture native` — native IR symbol model
- `./mfb spec linker symbols-and-relocations` — `_mfb_ifn_` / `_mfb_fn_` symbol namespaces
- `./mfb spec architecture ir` — IR symbol references
