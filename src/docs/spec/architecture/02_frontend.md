# Shared Front End

The shared front end: manifest loading, source discovery, name resolution, monomorphization, entry-point validation, and source-syntax checking.

Every build — executable or package — runs the same front end, from reading
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

Not every field of the project manifest format is enforced. The build
primarily consumes:

- `name`
- `version`
- `mfb`
- `kind`
- `sources[*].root`
- `entry`
- `author`
- `url`
- `packages`

Fields such as `include`, `exclude`, `role`, `targets`, and richer repository
metadata are documented for the project format but are not the active source of
build behavior in the compiler code reviewed here.[[src/manifest/mod.rs:validate_project_manifest]]

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

Before name resolution, the resolver runs the parsed AST
through a fixed chain of built-in source-package augmenters, each of which may
inject the package's MFBASIC source companion (and, for `json`, expand
`Json`-typed declarations) when the project uses that package. The order is
load-bearing:

```text
json -> csv -> regex -> datetime -> vector -> http -> net -> crypto -> encoding
```

`http` is augmented before `net` because `http`'s source companion
imports `net`; the `net` augmenter must see http's source
already present so the `net` dependency is detected and its companion injected.
For the same reason `crypto` is augmented before `encoding` (`crypto_package.mfb`
imports `encoding`). `vector` has no ordering dependency (it imports only the
intrinsic `math` package).
Each augmenter takes the previous augmenter's output, so the augmented AST that
reaches the resolver is the cumulative result of the whole chain. (The
`collections` package is injected earlier, during `parse_project`.)[[src/resolver/mod.rs:resolve_project_with]]

## Name Resolution

Name resolution is handled by the resolver.[[src/resolver/]]

The resolver has two jobs:

1. Collect top-level symbols from the project.
2. Validate references inside imports, type declarations, function bodies, and
   expressions.

The resolver knows the built-in type names:
`Boolean`, `Byte`, `Error`, `ErrorLoc`, `Fixed`, `Float`, `Integer`, `Json`,
`Nothing`, `Result`, `String`, plus the resource and record types contributed by
built-in packages — `File` (fs), `TermColor` and `TermSize` (term), `Socket`,
`Listener`, `Address`, `UdpSocket`, `Datagram`, `DatagramText` (net), and
`TlsSocket`/`TlsListener` (tls). The package-contributed names are referenced by a
shared constant so the resolver list and the packages stay in
sync.[[src/resolver/mod.rs:BUILTIN_TYPES]]

Before resolving, the resolver runs the built-in package augmentation chain
described above (see "Built-in Package Augmentation"), so the rest of resolution
sees the augmented AST.[[src/builtins/json.rs:augmented_project]]

It also reads declared package dependencies from the manifest and uses those to
validate imported package roots. For source imports, it detects duplicate
imports in a file, duplicate top-level names, duplicate function overloads with
the same parameter type shape, unknown types, unknown functions, invalid
constructors, invalid member references, and related symbol errors.

Resolution runs twice:

```text
parsed AST
  -> name resolution
  -> monomorphization
  -> name resolution (again)
```

The second resolution pass is important because monomorphization rewrites
generic/template code into concrete declarations that must also obey normal
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
The selected function must be a top-level `SUB` or `FUNC` with one of these
effective signatures:

```basic
SUB main
END SUB

SUB main(args AS List OF String)
END SUB

FUNC main AS Integer
END FUNC

FUNC main(args AS List OF String) AS Integer
END FUNC
```

Rules enforced by the implementation:

- A `FUNC` executable entry must return `Integer`.
- The entry may have zero parameters or one `List OF String` parameter.
- The args parameter must not declare a default value.
- Missing or invalid executable entries are compile-time errors.[[src/manifest/entry.rs:validate_entry_point]]

The resulting IR entry records the entry name, return type, and whether the
program accepts command-line arguments.

## Semantic Checking (two passes, one source of truth)

Semantic validation is **split by where the rule can be seen**. The
authoritative checker for every *semantic* rule is the IR semantic verifier,
which runs on the typed IR. It is the sole rejecter for
those rules on **both** paths: the freshly lowered IR of the program being
built, and the decoded IR of every imported `.mfp` package. A crafted package
never passed any source check, so running the same checker over its IR is what
keeps type-confused IR out of a victim's binary (see
`./mfb spec package verifier-rules`).[[src/ir/verify/]]

The front-end source-syntax checker retains only the
rules about **source syntax** — constructs that total lowering *erases*, so they
can never appear in IR or in a package: named-argument call binding
(`f(x := …)` duplicate/unknown names and the post-normalization arity/argument
shapes), `EXIT FUNC`/`EXIT SUB` flavor distinctions, inline-`TRAP` boundaries
and fallibility, lambda capture-escape analysis, and package-metadata ingestion
(`PACKAGE_INVALID`, thread-sendability, the native-`LINK` slot-level ABI spans).
It builds indices for local/package functions, user types and their kinds/fields,
union members, and enum members to evaluate those rules, and it models the
language's type forms (`./mfb spec language types`) to do so — but nothing
downstream consumes its inference; lowering re-infers independently. It does
**not** emit any rule in `ir::RELOCATED_TO_IR_VERIFY` — the 65 rules plan-20 moved
to the IR semantic verifier, which is their sole rejecter. That is enforced, not
merely intended: `SyntaxChecker::report` carries a `debug_assert!` that the rule
it is about to emit is not in that list, so a regression fails a debug build
rather than double-reporting.

It does still emit semantic rules of its own — the ones that were never
relocated, including the whole `NATIVE_*` and `TESTING_*` families, the
inline-`TRAP`/lambda/isolation rules, and a handful such as
`TYPE_CALL_ARGUMENT_MISMATCH` and `TYPE_CALL_ARITY_MISMATCH`. The dividing line
is membership in `RELOCATED_TO_IR_VERIFY`, not the distinction between "syntax"
and "semantics". [[src/syntaxcheck/]] [[src/ir/mod.rs:RELOCATED_TO_IR_VERIFY]]

The build runs both over the source program: the source-syntax checker
gathers the syntax-rule diagnostics, IR is lowered, the IR semantic verifier
gathers the semantic-rule diagnostics, and the two streams are concatenated in
source order and rendered together. On the package path, package merging runs
the IR semantic verifier once over the fully merged IR before any code is emitted.
[[src/cli/build.rs:build_project]] [[src/target/shared/nir/lower.rs:merge_packages]]

## See Also

* ./mfb spec language types — the source-level type model the checkers share
* ./mfb spec package verifier-rules — the package-path semantic verifier
* ./mfb spec diagnostics rule-codes — the diagnostic rule registry
* ./mfb spec tooling source-selection — the glob-matching algorithm behind source-file selection
* ./mfb spec architecture monomorphization — the template-expansion pass run between the two resolution passes
