# Shared Front End

The shared front end: manifest loading, source discovery, name resolution, monomorphization, entry-point validation, and type checking.

Every build — executable or package — runs the same front end, from reading
`project.json` through type checking, before the pipeline splits at IR.

## Project Manifest Loading

The project manifest is `project.json` in the build location. The manifest is
read and validated by `validate_project_manifest` in `src/main.rs`.

The current implementation requires these string fields:

- `name`
- `version`
- `mfb`

It also requires `sources` to be a non-empty array of objects, each with a
string `root` field. The `kind` field is also required and must be a string;
it is expected to be `"executable"` or `"package"`, and unknown kinds are
diagnosed (the validator continues after that diagnostic). Optional `entry`,
`author`, and `url` fields must be strings when present.

The current implementation does not enforce every field of the project manifest
format. In particular, it primarily consumes:

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
build behavior in the compiler code reviewed here.

## Source Discovery and Parsing

Source parsing is implemented in `src/ast.rs`.

`ast::parse_project` receives the validated project name, project directory,
and manifest. It reads `sources[*].root`, joins each root to the project
directory, recursively collects `.mfb` files, sorts them, and parses each file.

For each source file:

1. The file is read as text.
2. `lexer::lex` tokenizes the source.
3. `FileParser` parses tokens into an `AstFile`.
4. The file path is stored relative to the project directory.

The AST keeps imports, type declarations, function declarations, statements,
expressions, visibility, parameters, default values, and source line positions.
`mfb build -ast` serializes this structure to `<project>.ast`.

Current discovery behavior:

- If a source root is a file, it is included only when its extension is `.mfb`.
- If a source root is a directory, all nested `.mfb` files are included.
- Empty source roots are compile-time errors.
- Per-root `include`/`exclude` glob patterns are applied by the source collector
  (`matches_source_patterns` in `src/ast.rs`). When unspecified, `include`
  defaults to `["**/*.mfb"]` and `exclude` defaults to empty, so every nested
  `.mfb` file is collected by default.

## Name Resolution

Name resolution is implemented in `src/resolver.rs`.

The resolver has two jobs:

1. Collect top-level symbols from the project.
2. Validate references inside imports, type declarations, function bodies, and
   expressions.

The resolver knows the built-in type names in `BUILTIN_TYPES` (`src/resolver.rs`):
`Boolean`, `Byte`, `Error`, `ErrorLoc`, `Fixed`, `Float`, `Integer`, `Json`,
`Nothing`, `Result`, `String`, plus the resource and record types contributed by
built-in packages — `File` (fs), `TermColor` and `TermSize` (term), `Socket`,
`Listener`, `Address`, `UdpSocket`, `Datagram`, `DatagramText` (net), and
`TlsSocket` (tls). The package-contributed names are referenced by constant
(e.g. `builtins::fs::FILE_TYPE`) so the resolver list and the packages stay in
sync.

Before resolving, the resolver calls `builtins::json::augmented_project` to
expand `Json`-typed declarations into the augmented AST used for the rest of
resolution.

It also reads declared package dependencies from the manifest and uses those to
validate imported package roots. For source imports, it detects duplicate
imports in a file, duplicate top-level names, duplicate function overloads with
the same parameter type shape, unknown types, unknown functions, invalid
constructors, invalid member references, and related symbol errors.

Resolution runs twice:

```text
parsed AST
  -> resolver::resolve_project
  -> monomorph::monomorphize_project
  -> resolver::resolve_project again
```

The second resolution pass is important because monomorphization rewrites
generic/template code into concrete declarations that must also obey normal
symbol rules.

## Monomorphization

Monomorphization is implemented in `src/monomorph.rs`.

This pass takes the parsed, initially resolved AST and produces a concrete AST.
Template/generic declarations are expanded into concrete forms based on use
sites. The rest of the pipeline consumes the concrete AST, not the original AST.

Because the concrete AST introduces generated declarations and names, the build
pipeline immediately runs the resolver again after monomorphization.

## Entry-Point Validation

Entry-point validation is implemented in `validate_entry_point` in
`src/main.rs`.

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
- Missing or invalid executable entries are compile-time errors.

The resulting IR entry records the entry name, return type, and whether the
program accepts command-line arguments.

## Type Checking

Type checking is implemented in `src/typecheck.rs`.

The type checker builds indices for:

- Local project functions.
- Exported package functions.
- User-defined types.
- Type kinds.
- Type fields.
- Union member types.
- Enum members.

It then validates declarations, statement flow, expression types, mutability,
constructor usage, member access, function calls, built-in calls, package calls,
return/fail behavior, isolated-function restrictions, and default values.

The type model includes primitive and compound forms:

- `Boolean`
- `Byte`
- `Error`
- `ErrorLoc`
- `Fixed`
- `Float`
- `Integer`
- `List(T)`
- `Map(K, V)`
- `Res(T)` — a `RES`-marked resource element of a collection (`List OF RES File`,
  `Map ... TO RES File`); the collection holds a borrow and owns nothing
- function values (with parameter types, return type, and isolated flag)
- `Nothing`
- `Result(T)`
- `String`
- `Thread(message, optional resource, output)` — the optional middle slot is the
  resource-plane type carried by `thread::transfer`/`accept`; `None` for a
  data-only thread
- `ThreadWorker(message, optional resource, output)`
- user-defined types (`User`)
- `Unknown` — the inference placeholder for an as-yet-undetermined type

Type checking is the last front-end validation pass before lowering to IR.
