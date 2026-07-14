# Audit 1 — Compiler Frontend (lexer/parser/resolver/typecheck/monomorph)

Code-grounded security audit of the MFBASIC frontend: `src/lexer.rs`,
`src/numeric.rs`, `src/ast/*`, `src/resolver/*`, `src/typecheck/*`,
`src/monomorph/*`. Threat model: `.mfb` source is semi-trusted (developer's own),
but (1) imported package bodies are decoded to AST/IR and flow through these
stages, and (2) a compiler that panics/aborts/hangs on crafted input is a DoS and
a marker for latent codegen-correctness bugs.

All findings below were reproduced against a freshly built `target/debug/mfb`
using real MFBASIC syntax. Reproductions ran in `/tmp/mfbtest` with a standard
`project.json` (executable, `src/main.mfb`, `mfb build -ast .`).

Summary by severity:
- HIGH: FE-01 (parser recursion stack overflow), FE-02 (monomorph polymorphic-
  recursion blowup), FE-03 (statement-block recursion stack overflow).
- MEDIUM: FE-04 (unbounded `Vec::with_capacity` on attacker-controlled package
  count).
- LOW: FE-05 (bare Float literal overflow to `inf` not range-checked).

---

## FE-01 — HIGH: Unbounded recursive-descent expression parsing → native stack overflow (SIGABRT)

**Location:** `src/ast/expr.rs:4` (`parse_expression` → `parse_pipeline` → … →
`parse_primary`), specifically the mutual recursion at `src/ast/expr.rs:422-426`
(grouped `(expr)`), `:427` (`parse_list_literal`), `:307` (`parse_argument_list`),
and `:699` (`parse_map_literal`).

**Issue:** The recursive-descent expression parser has **no depth guard**. A
grouped expression re-enters `parse_expression` unboundedly:

```rust
// src/ast/expr.rs:422
TokenKind::LParen => {
    let expression = self.parse_expression();   // recurses with no depth limit
    self.consume_kind(TokenKind::RParen, "Expected `)` after expression.");
    expression
}
```

The same holds for `[` list literals (`parse_list_literal` at `:685` →
`parse_expression`), map literals, and call/constructor argument lists. A
`grep` for `recursion|depth|RECURSION_LIMIT|MAX_DEPTH` across `src/` finds only
`src/escape.rs` block-nesting counters — there is no recursion limit anywhere in
the lexer/parser/resolver/typecheck. Each frame consumes native stack; deep
nesting overflows the thread stack and the process aborts (`fatal runtime error:
stack overflow, aborting`), a hard crash, not a diagnostic.

**Trigger:** ~50k nested parentheses in a single expression (reproduced):

```basic
IMPORT io
SUB main()
  LET x AS Integer = (((( … 50000 deep … 1 … ))))
  io::print("ok")
END SUB
```

Observed: `thread 'main' has overflowed its stack / fatal runtime error: stack
overflow, aborting`. `[[[[…]]]]` list nesting and deeply chained calls trigger the
same path.

**Fix:** Add a recursion-depth counter to `FileParser` (a `depth: usize` field,
or an explicit `RecursionGuard` RAII helper). Increment on entry to
`parse_expression` (and the type-name recursion in `parse_type_name`), decrement
on exit, and when it exceeds a fixed bound (e.g. 256) emit a
`MFB_PARSE_EXPRESSION_TOO_DEEP` diagnostic via `self.report(...)` and return
`None` instead of recursing further. This changes no language semantics — real
programs never approach the limit; it only converts an abort into a clean error.
(The same guard field should cover `parse_type_name`, which recurses on
`List OF …`, `Map OF … TO …`, and function-type params.)

---

## FE-02 — HIGH: Monomorph polymorphic-recursion → unbounded type instantiation → stack overflow (SIGABRT)

**Location:** `src/monomorph/lower.rs:1217` (`concrete_type_name`) →
`:1288` (`self.instantiate_type(&name, &args)`) → `:524` (`instantiate_type`) →
`:539` (`self.lower_type(...)` lowers each field back through
`concrete_type_name`). The dedup guard at `:529` is ineffective for this case.

**Issue:** A user generic `TYPE` whose field references the *same* generic type
with a **strictly larger** type argument (polymorphic recursion) instantiates
infinitely. Instantiating `Node OF Integer` lowers its field
`child AS Node OF List OF T`, which calls `concrete_type_name("Node OF List OF
Integer")` → `instantiate_type("Node", ["List OF Integer"])`, whose field is
`Node OF List OF List OF Integer`, and so on without bound. The memoization key:

```rust
// src/monomorph/lower.rs:528
let key = format!("{name}<{}>", args.join(","));
if !self.emitted_type_keys.insert(key) {   // never trips: each key is distinct
    return concrete_name;
}
```

never repeats, because every level has a strictly larger argument
(`List OF Integer`, `List OF List OF Integer`, …). There is no depth cap and no
"seen this template with a growing argument" cycle check, so instantiation
recurses until the native stack overflows and the process aborts. This is a
compile-time DoS that is reachable through an imported package too (templates are
instantiated by the importing compilation, per
`src/docs/spec/language/03_templates.md:39`).

**Trigger:** Reproduced (aborts with stack overflow):

```basic
IMPORT io

TYPE Node OF T
  value AS T
  child AS Node OF List OF T
END TYPE

SUB take(n AS Node OF Integer)
END SUB

SUB main()
  io::print("ok")
END SUB
```

The single `SUB take(n AS Node OF Integer)` parameter type forces one
instantiation, which then diverges. (`next` is a reserved keyword, so the field
is named `child`.)

**Fix:** Bound instantiation. Add an instantiation-depth counter (or a work-stack
depth) threaded through `instantiate_type`/`concrete_type_name`; when it exceeds a
fixed limit (e.g. 64 levels of nesting for one template family) emit a new
`TYPE_TEMPLATE_INSTANTIATION_TOO_DEEP` diagnostic (via `self.report`) and stop.
A more precise check: detect that the argument being instantiated for template
`name` strictly *contains* an earlier pending argument for the same `name`
(structural growth) and reject as non-terminating. Either is an implementation-
only change (polymorphic recursion is not a supported feature — the spec says
"monomorphized templates, not runtime generics"), so rejecting it changes no
valid program's behavior. Apply the same guard to `instantiate_function`
(`:356`), which can diverge analogously through recursive template calls.

---

## FE-03 — HIGH: Unbounded nested-statement-block parsing → native stack overflow (SIGABRT)

**Location:** `src/ast/stmt.rs:4` (`parse_statement`) and `:692`
(`parse_statement_block`). `parse_statement` recurses into
`parse_statement_block` for each `IF`/`WHILE`/`FOR`/`FOREACH`/`MATCH`/`DO` body,
which parses statements, each of which may open another block — with no depth
guard.

**Issue:** Same root cause as FE-01 but a distinct recursion path (statement
nesting rather than expression nesting). Deeply nested control-flow blocks
overflow the native stack. No depth limit exists on the statement parser.

**Trigger:** ~20k nested `IF TRUE THEN … END IF` blocks (reproduced):

```basic
IMPORT io
SUB main()
  IF TRUE THEN
  IF TRUE THEN
    … 20000 deep …
    io::print("deep")
  END IF
  END IF
END SUB
```

Observed: `fatal runtime error: stack overflow, aborting`.

**Fix:** Share the depth counter added for FE-01. Increment on entry to
`parse_statement_block` (block nesting), cap at a fixed bound, and emit a
`MFB_PARSE_BLOCK_TOO_DEEP` diagnostic and return the partial block instead of
recursing. No semantic change. Note the resolver (`resolve_statement` /
`resolve_expression` in `src/resolver/resolution.rs:762`/`:604`) and the
typecheck/monomorph walks recurse over the same tree; capping parse depth caps
all downstream tree depth, so a single parser-side guard removes the whole class.

---

## FE-04 — MEDIUM: Unbounded `Vec::with_capacity` on attacker-controlled package section count → allocation DoS (abort)

**Location:** `src/binary_repr/reader.rs:67-68`, and the same pattern at
`:424-425`, `:437-438`, `:442-443`, `:464-465`, `:514-515` (and peers). This is
the compiled-`.mfp` package decoder — directly on the "imported package bodies
are decoded" attack surface named in the threat model, feeding AST back into the
frontend.

**Issue:** A `u32` count is read from the package file and used to pre-allocate a
`Vec` *before any element is read or the count is validated against the remaining
byte length*:

```rust
// src/binary_repr/reader.rs:67
let count = cursor_u32(bytes, &mut offset)? as usize;
let mut decls = Vec::with_capacity(count);   // count up to 0xFFFF_FFFF, unchecked
for _ in 0..count { … }
```

A crafted `.mfp` supplying `count = 0xFFFF_FFFF` forces
`Vec::with_capacity(~4e9 * size_of::<T>())`, which aborts on allocation failure
(or thrashes/OOMs). Every `cursor_u32(...) as usize; Vec::with_capacity(count)`
site listed above is affected. (The header section-count site at `:282` *is*
bounds-checked at `:285` against the entry table — good; the payload-level counts
above are not.)

**Trigger:** Craft/patch a `.mfp` so a section payload's leading `u32` element
count is `0xFFFFFFFF`, then `mfb pkg info <pkg>` / `mfb build` on a project that
imports it. (No `.mfb` source needed — the malicious input is the package file.)

**Fix:** Do not trust the count for allocation. Either (a) replace
`Vec::with_capacity(count)` with `Vec::new()` and let it grow as elements are
successfully decoded, or (b) validate `count` against a conservative upper bound
derived from the remaining payload length (each element needs ≥1 byte, so
`count <= bytes.len() - offset`) and return `None`/decode-error when it exceeds
that. Option (a) is the minimal, allocation-safe fix and matches the existing
element-by-element `cursor_*` decode loop, which already returns `None` on a short
buffer. (Out of the strict lexer→monomorph scope, but on the package-decode
surface the threat model calls out; recorded here for completeness.)

---

## FE-05 — LOW: Bare (untyped) Float literal overflowing to `inf` is not range-checked

**Location:** `src/typecheck/inference.rs:34-48` (bare `Number` inference) and
`src/typecheck/checking.rs:230-232` (range check only runs when a type is
*declared*).

**Issue:** Integer literals are range-checked unconditionally
(`inference.rs:37`: `value.parse::<i64>().is_ok()` else
`TYPE_INTEGER_LITERAL_OVERFLOW`). Float literals are **only** range-checked when
there is an explicit expected/declared type:

```rust
// src/typecheck/checking.rs:230
let reported_range_error = declared.zip(value).is_some_and(|(declared, value)| {
    self.report_primitive_literal_range_error(declared, value, file, line)
});
```

`report_primitive_literal_range_error` (`src/typecheck/mod.rs:2072`) only fires
for `Type::Float`/`Byte`/`Fixed` when `declared` is `Some`. For a bare
`LET x = <huge>.0` (no `AS Float`), `declared` is `None`, so the Float overflow
check never runs. The literal string carries through to codegen where
`native_immediate_value` (`src/target/shared/code/type_utils.rs:272`) does
`value.parse::<f64>()...to_bits()`; a very long digit string parses to `f64::INFINITY`
(not an error), so the program silently gets `inf`. Not a crash and not
memory-unsafe, but a silent-wrong-value correctness gap relative to the
typed-literal path, which *does* reject it (`TYPE_FLOAT_LITERAL_OVERFLOW`).

**Trigger:** `LET x = 1` followed by 400 zeros then `.0` (a Float literal, since
it contains `.`) with no `AS Float` annotation — accepted; codegen stores `inf`.
(MFBASIC's number lexer, `src/lexer.rs:342`, accepts only `digits.digits` with no
`e` exponent, so the overflow needs a long digit run rather than `1e999`.)

**Fix:** In `infer_expression` for `Expression::Number(value)` where
`value.contains('.')` (`src/typecheck/inference.rs:35`), also run the finite-range
check (`text.parse::<f64>()` then `is_finite()`, mirroring
`float_literal_range_error` in `src/typecheck/helpers.rs:38`) and emit
`TYPE_FLOAT_LITERAL_OVERFLOW` when the value is non-finite, regardless of whether
a type was declared. This aligns the bare-literal path with the already-existing
typed-literal check; no new syntax or semantics.

---

## Checked and OK

- **String escape handling (`src/lexer.rs:313-326`).** MFBASIC strings support
  only `\"`, `\\`, `\n`, `\t`, and "unknown escape → literal char"; there is **no**
  `\x`/`\u`/`\{...}` code-point syntax, so the malformed-escape / surrogate /
  out-of-range-code-point crash classes do not exist here. `\` at EOF `break`s
  cleanly and reports an unterminated-string diagnostic. (Note: `src/escape.rs`
  despite its name is *resource* escape analysis, not string escapes.)
- **`Vec<char>` lexer indexing (`src/lexer.rs:577` `peek`, `:581` `peek_next`).**
  `peek` is only called after `is_at_end()` guards in `lex_all`; `peek_next` uses
  `.get(...)` (returns `Option`). No reachable out-of-bounds index found.
- **Integer literal overflow.** Caught: bare (`inference.rs:37`), negated
  (`inference.rs:190` via `integer_literal_in_range`), and typed Byte/Fixed
  (`typecheck/helpers.rs`) all report `*_LITERAL_OVERFLOW`. `Fixed` decimal parse
  uses `checked_mul`/`checked_add` (`type_utils.rs:307-313`).
- **Parser token indexing.** `peek()` (`parser.rs:256`) indexes
  `self.tokens[self.current]`; the lexer always appends a trailing `Eof`
  (`lexer.rs:275`) and `advance()` (`:248`) refuses to step past end via
  `is_at_end()`, so `current` stays in-bounds. `previous()` (`:263`) is only
  reached after an `advance()`, so `current >= 1`.
- **`.expect`/`unreachable!` in parser (`stmt.rs:323`, `items.rs:208`,
  `parser.rs` `unreachable!` in operator maps).** All guarded by a preceding
  `is_none()` check or an exhaustive discriminant match; not attacker-reachable.
- **Overload resolution ambiguity (`monomorph/lower.rs:422-480`).** A return-type
  overload set with no unique expected type reports `TYPE_OVERLOAD_AMBIGUOUS`
  and returns `None` rather than silently picking a wrong callee.
- **Internal-name forgery.** `__`-prefixed identifiers are rewritten to an
  untypeable sigil only in internal (built-in) files (`lexer.rs:410`); user code
  cannot forge `#`-sigil internal names.
