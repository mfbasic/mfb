# Shared Front End

The shared front end: manifest loading, source discovery, name resolution, monomorphization, entry-point validation, and source-syntax checking.

Every build — executable or package — runs the shared front end, from reading
`project.json` through source-syntax checking, before the pipeline splits at IR.
(The bulk of semantic validation runs *after* lowering, on the typed IR —
see "Semantic Checking" below.)

## Project Manifest Loading

The project manifest is `project.json` in the build location. The manifest is
read and validated by the compiler's manifest loader.

The manifest requires these string fields:

- `name`
- `version`
- `mfb`

It also requires `sources` to be a non-empty array of objects, each with a
string `root` field. The `kind` field is also required and must be a string;
it is expected to be `"executable"` or `"package"`, and unknown kinds are
diagnosed (the validator continues after that diagnostic). Optional `entry`,
`author`, and `url` fields must be strings when present.

Source selection consumes `sources[*].root`, `include`, and `exclude`.
Build orchestration also consumes dependency declarations, app mode and icon,
resource entries, native-library locators, and runtime configuration. The field
schema and validation rules are owned by `./mfb spec tooling project-manifest`.
[[src/manifest/mod.rs:validate_project_manifest]] [[src/cli/build/mod.rs:build_project]]

## Source Discovery and Parsing

Source parsing is handled by the parser.[[src/ast/]]

The parser receives the validated project name, project directory,
and manifest. It reads `sources[*].root`, joins each root to the project
directory, recursively collects `.mfb` files, sorts them, and parses each file.

For each source file:

1. The file is read as text.
2. The lexer tokenizes the source.
3. The parser turns the tokens into the file's AST.
4. The file path is stored relative to the project directory.

The AST keeps imports, type declarations, function declarations, statements,
expressions, visibility, parameters, default values, and source line positions.
`mfb build --ast` serializes this structure to `<project>.ast`.

Current discovery behavior:

- If a source root is a file, it is included only when its extension is `.mfb`.
- If a source root is a directory, all nested `.mfb` files are included.
- Empty source roots are compile-time errors.
- Per-root `include`/`exclude` glob patterns are applied by the source collector.
  When unspecified, `include`
  defaults to `["**/*.mfb"]` and `exclude` defaults to empty, so every nested
  `.mfb` file is collected by default.[[src/ast/manifest.rs:matches_source_patterns]]

## Source-File Selection

The source-file selector enumerates the on-disk `.mfb` files that a
build sees. For each manifest `sources[*]` entry it walks the joined source
root, then keeps each file whose project-relative path satisfies the entry's
`include`/`exclude` glob patterns. When unspecified,
`include` defaults to `["**/*.mfb"]` and `exclude` is empty, so every nested
`.mfb` file is collected. Results are returned in a stable, sorted order. The
same selection feeds both AST builds and the raw-text tools such as `mfb fmt`.
The full glob-matching algorithm is
`./mfb spec tooling source-selection`.[[src/ast/manifest.rs:collect_selected_source_files]]

## Compiler-Owned Prelude

The parser appends one synthetic file, the built-in prelude, after
all selected user sources. Its path is the sentinel
`"<builtin prelude>"`, and it is appended last so the user's first source file
remains `files[0]` — the monomorphizer emits generated instantiations into that
first file.[[src/ast/manifest.rs:builtin_prelude_file]]

The prelude declares the always-in-scope generic record templates
`Pair OF A, B` (fields `first AS A`, `second AS B`) and `Partition OF T` (fields
`matched AS List OF T`, `unmatched AS List OF T`). Both are exported, ordinary
generic records — constructible, field-accessible, copyable, and thread-sendable
when their members are — handled by the normal template machinery rather than
special-cased.

The AST serializer filters this file out by path, so the prelude does not
appear in `mfb build --ast` golden output. The resolver, monomorphizer, and both
checkers consume the full project (prelude included), so `Pair` and
`Partition` resolve, monomorphize, and check as if user-declared.[[src/ast/manifest.rs:BUILTIN_PRELUDE_PATH]]

## Built-in Package Augmentation

Before name resolution, the resolver runs the parsed AST through a fixed chain that
injects built-in packages' MFBASIC source.
[[src/resolver/mod.rs:augment_project]] [[src/resolver/mod.rs:resolve_project_with]]

The chain has two stages:

1. **The registry's generic pass** injects the source companion of every registry package the program
   imports, and each gated helper whose gate the program opens. This covers `json`, `csv`, `regex`,
   `datetime`, `vector`, `crypto`, `canvas` and most other packages.
   [[src/codegen/registry/mod.rs:augment_project]]
2. **Late passes, in a fixed order:**

   ```text
   http -> net -> encoding -> color -> compress
   ```

   The order is load-bearing. The generic pass reads the pre-injection AST, so it cannot see a package
   that only another builtin's injected source uses. So each late pass sees the source the passes before
   it added:
   - `http`'s companion imports `net`, so `net` follows `http`;
   - crypto's injected source imports `encoding`, so `encoding` follows it (the strings scalar seam no
     longer does: `toScalars`/`fromScalars` are lowered natively since bug-772);
   - canvas's companion imports `color`, so `color` follows the generic pass;
   - canvas's PNG decoder calls `compress::zlibDecode`, so `compress` comes last.

   A late pass injects the package's companion plus the gated helpers its own view opens, and skips a file
   the project already has, so nothing is injected twice. [[src/codegen/registry/mod.rs:late_pass_files]]

Each pass takes the previous one's output, so the AST elaborated for resolution is the cumulative result.
The `collections` package is injected earlier, during `parse_project`, because the monomorphizer needs its
generic source.

## Name Resolution

Name resolution is handled by the resolver.[[src/resolver/]]

The resolver has two jobs:

1. Collect top-level symbols from the project.
2. Validate references inside imports, type declarations, function bodies, and
   expressions.

The resolver seeds its type set with the primitive names and package-qualified
record/resource identities in `BUILTIN_TYPES`, including `Money`, `Scalar`,
`AttributedString`, and the `fs`, `color`, `term`, `net`, `tcp`, `udp`, `tls`,
`audio`, `json`, and `process` types. Package type identities do not become bare
names in unrelated files. [[src/resolver/mod.rs:BUILTIN_TYPES]]

Before resolving, the resolver runs the built-in package augmentation chain
described above (see "Built-in Package Augmentation"), so the rest of resolution
sees HIR elaborated from the augmented AST.[[src/resolver/mod.rs:augment_project]]

It also reads declared package dependencies from the manifest and uses those to
validate imported package roots. For source imports, it detects duplicate
imports in a file, duplicate top-level names, duplicate function overloads with
the same parameter type shape, unknown types, unknown functions, invalid
constructors, invalid member references, and related symbol errors.

Resolution runs twice:

```text
parsed AST
  -> builtin augmentation and HIR elaboration for initial resolution
  -> augmented generic HIR
  -> monomorphization to concrete HIR
  -> name resolution (again, without DOC revalidation)
```

The second resolution pass is important because monomorphization rewrites
generic/template HIR into concrete declarations that must also obey normal
symbol rules.[[src/monomorph/mod.rs:monomorphize_project]]

## Monomorphization

The monomorphization pass expands template/generic
declarations into concrete declarations between the two resolution passes; see
`./mfb spec architecture monomorphization`.[[src/monomorph/]]

## Entry-Point Validation

Entry-point validation is handled by the entry-point validator.

Package projects have no executable entry point and return `None` for the IR
entry.

Executable projects use the manifest `entry` field, defaulting to `main`.
The selected function must be a top-level `FUNC` with one of these
effective signatures:

```basic
FUNC main AS Nothing
END FUNC

FUNC main(args AS List OF String) AS Nothing
END FUNC

FUNC main AS Integer
END FUNC

FUNC main(args AS List OF String) AS Integer
END FUNC
```

Rules enforced by the implementation:

- A `FUNC` executable entry must return `Integer` or `Nothing`.
- The entry may have zero parameters or one `List OF String` parameter.
- The args parameter must not declare a default value.
- Missing or invalid executable entries are compile-time errors.[[src/manifest/entry.rs:validate_entry_point]]

The resulting IR entry records the entry name, return type, and whether the
program accepts command-line arguments.

## Where a type stops being a string

The parser stores a type annotation as a flat spelling ([architecture
type-name-encoding](./mfb spec architecture type-name-encoding)). `hir::elaborate`
is the boundary where that spelling is parsed, into a `ParameterType`; from there
forward — resolver, monomorphization, the shape pass, IR lowering, the IR
semantic verifier, the `TypeModel` builder and every code emitter — the compiler
carries the variant tree and matches on it. Post-elaboration type consumers do not re-derive
structure by stripping `"List OF "` or splitting on `" TO "`.

This is a checked property, not a convention: `tests/guards/no_type_strings.rs` scans
the tree for eight needle classes (a `ParameterType::parse` call, a type-as-`&str`
parameter, a `match` arm or `==` against a type spelling, a hand-rolled grammar
op, a `format!`-built spelling, a `String`-keyed type map, a second grammar) and
fails if one appears outside the closed boundary list. The boundaries are the
parser itself, the AST→HIR seam, the `.mfp` type codec, the IR binary codec and
the package manifest — every one a place that genuinely converts between the two
worlds. [[src/hir/build.rs]] [[tests/guards/no_type_strings.rs]]

## Semantic Checking (two passes, one source of truth)

Semantic validation is **split by where the rule can be seen**. The
authoritative checker for every *semantic* rule is the IR semantic verifier,
which runs on the typed IR. It is the sole rejecter for
those rules on **both** paths: the freshly lowered IR of the program being
built, and the decoded IR of every imported `.mfp` package. A crafted package
never passed any source check, so running the same checker over its IR is what
keeps type-confused IR out of a victim's binary (see
`./mfb spec package verifier-rules`).[[src/ir/verify/]]

The pre-lowering **shape pass** (`ir::shape`) holds the only rules that cannot
live in the IR verifier: the ones whose evidence total lowering *erases*, so
that the fact never reaches the IR or a package — named-argument binding
(`f(x := …)` duplicate/unknown names and the omission/count/argument-type
shapes lowering normalizes away), `EXIT FUNC` and the statements it leaves
unreachable, inline-`TRAP` boundaries (`RECOVER`
placement and count, a handler's fall-through edge), the checker's "unknown
value" cascade, the `TESTING` assertion builtins' argument rules (lowering
expands the assertion into comparisons + `FAIL`), a `Money` scaled by a bare
decimal literal (the literal's spelling), the `Error`/`ErrorLoc` constructor
form (lowering synthesizes `Constructor{Error}` itself), a native `CONST` pin
that does not fold and a `FREE` block's deallocator signature, the `STATE`
type of a top-level binding or a lambda parameter (`TYPE_STATE_INVALID` —
lowering carries neither clause into the IR; see
`./mfb spec language resource-management`), and the
imported-package metadata validation (`PACKAGE_INVALID`). It walks the
concrete (post-monomorph) HIR with lowering's own typing oracle
(`ir::lower::expression_type`) and tracks scopes exactly as lowering binds them,
so it carries no inference of its own; every rule it emits is annotated with the
erased fact that justifies it. [[src/ir/shape.rs]]

The build runs both over the source program: the shape pass gathers its
diagnostics, IR is lowered, the IR semantic verifier gathers the rest, and the
two streams are concatenated (shape's first) and rendered together. On the
package path, package merging runs the IR semantic verifier once over the fully
merged IR before any code is emitted.
[[src/cli/build/mod.rs:build_project]] [[src/target/shared/nir/lower.rs:merge_packages]]

## See Also

* ./mfb spec language types — the source-level type model the checkers share
* ./mfb spec package verifier-rules — the package-path semantic verifier
* ./mfb spec diagnostics rule-codes — the diagnostic rule registry
* ./mfb spec tooling source-selection — the glob-matching algorithm behind source-file selection
* ./mfb spec architecture monomorphization — the template-expansion pass run between the two resolution passes
