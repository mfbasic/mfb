# 13. Modules & Packages

**Project source = one package.** The `.mfb` files selected by the current
project's `project.json` together form that project's source package.
Directories inside a source root do not create package boundaries or package
namespaces. Additional packages are introduced only through the importing
project's `project.json` `packages` array.

Visibility (`Visibility` enum in `src/ast.rs`; default is `Private`):
- `PRIVATE` (default) — file-local.
- `PACKAGE` — visible to all files in the same package, hidden from importers.
- `EXPORT` — visible to importers.

Within a single build, `PACKAGE` and `EXPORT` are treated **identically** by
`visible_from` (`src/resolver.rs`, `src/typecheck.rs`): both are visible across
files in the project, and only `PRIVATE` is file-local. The `PACKAGE`/`EXPORT`
distinction matters only for what is written into the compiled `.mfp` package
(the exported-symbol flag), not for in-project name resolution. This is why a
cross-file reference to a `PRIVATE` declaration fails unless the declaration is
`PACKAGE`/`EXPORT` or the project is built as a single file.

Top-level `LET`, `MUT`, `FUNC`, `SUB`, `TYPE`, `UNION`, and `ENUM` may use `PRIVATE`, `PACKAGE`, or `EXPORT`. Fields in `TYPE` declarations may also use `PRIVATE`, `PACKAGE`, or `EXPORT`; omitted field visibility defaults to `EXPORT` when the containing type is `EXPORT`, otherwise to `PACKAGE` (`effective_field_visibility`, `src/typecheck.rs`) — i.e. the containing type's visibility, capped at `PACKAGE` for non-exported types.

Only exported top-level `FUNC` declarations may use `ISOLATED`. Imported package constructors are addressed as `package::identifier` when constructing values, but constructors for records with hidden fields are callable only from scopes that can see every required field.

Exported top-level `MUT` is allowed only when written explicitly as `EXPORT MUT`; it is package state visible to importers and must be surfaced by audit tooling. A top-level `MUT` without `EXPORT` is private or package-local according to its visibility annotation and remains discouraged for shared state.

Double-colon notation is reserved for package access. Dot notation is reserved for field access into data values and enum members:

```basic
IMPORT shapes
IMPORT longPackageName AS shortName

LET s = shapes::Circle[2.0]
io::print(toString(shapes::area(s)))
io::print(toString(s.radius))
```

Rules:

- A package-qualified name has exactly two parts: `package::identifier`.
- Nested package qualifiers are illegal: `a::b::c` is a compile error.
- Record fields use `value.field`. Methods and object-style access do not exist.
- Imports are not transitive. A package cannot export an imported package or create re-export chains.
- `IMPORT packageName AS aliasName` binds the package to `aliasName` in the importing file. The original package name is not also introduced by that import; use a second import only if both names are needed.
- An import alias must not conflict with another imported package name or alias, a top-level declaration visible in the file, or a built-in package name. The built-in packages (`is_builtin_import`, `src/builtins/mod.rs`) are: `collections`, `csv`, `datetime`, `errorCode`, `fs`, `http`, `io`, `json`, `math`, `net`, `regex`, `strings`, `term`, `thread`, and `tls`.
- `[visibility] FUNC alias AS qualified::name` declares a **function alias**. This form exists today only as a transparent re-export of a native `LINK` function: the resolver requires the alias target to resolve to a `LINK` (native) function signature (`link_target_signature`, `src/resolver.rs`) and reports `SYMBOL_UNKNOWN_IDENTIFIER` otherwise. It is not a general mechanism for aliasing arbitrary MFBASIC functions to package-qualified names.

```basic
' shapes package source
EXPORT FUNC area(s AS Shape) AS Float
EXPORT ISOLATED FUNC worker(path AS String) AS Integer
PRIVATE FUNC helper() AS Float
```

```basic
' main.mfb
IMPORT mathstuff
IMPORT shapes

io::print(toString(shapes::area(shapes::Circle[2.0])))
```

The import graph is resolved at compile time. Imports are not transitive: when
the compiler installs a dependency it reads only that package's exported type
names (`install_package_type_names`, `src/resolver.rs`) and never re-processes
the dependency's own imports, so transitivity does not occur structurally. (Note
to implementers: the compiler does **not** currently detect or diagnose an
import *cycle* — there is no circular-dependency check on the import graph;
recursive-record cycle detection is unrelated.)

`IMPORT packageName` resolves a package, not an arbitrary source file. The
compiler resolves the first identifier in the import using this order
(`resolve_imported_package`, `src/resolver.rs`):

1. A built-in package supplied by the toolchain, such as `io` (`is_builtin_import`);
   resolution returns immediately.
2. Otherwise the package **must** appear by `name` in the importing project's own
   `project.json` `packages` array. If no such dependency is declared,
   resolution fails immediately with `IMPORT_PACKAGE_NOT_DECLARED` — a
   non-built-in, undeclared package never falls through to the filesystem-probing
   steps below.
3. If the declared dependency has a `source` beginning with `local://`, the
   remainder of the value must be an absolute path (so the usual form is
   `local:///absolute/path`, which strips to `/absolute/path`). The compiler
   checks `/absolute/path/project.json`; the manifest `name` must match the
   import and `kind` must be `package`. A non-absolute remainder (e.g.
   `local://relative`) is an `IMPORT_LOCAL_PATH_INVALID` error. A package that
   uses `local://` cannot be released without replacing that dependency source.
4. Otherwise (declared dependency, no `local://` source), the compiler checks
   `<project_root>/packages/packageName.mfp`.
5. If no `.mfp` exists, the compiler checks
   `<project_root>/packages/packageName/project.json`; the manifest `name` must
   match the import and `kind` must be `package`.
6. Otherwise, the declared package is missing from the package store and the
   import is a compile-time error (`IMPORT_PACKAGE_NOT_INSTALLED`).

`<project_root>/packages` is the resolved dependency store, similar in role to
`node_modules` in Node projects. It is managed by the package manager. The
compiler does not implicitly import undeclared packages from this directory.
Each package is responsible for declaring its own dependencies; dependency
declarations are not inherited from importers and imports are not transitive.

## 13.1 Package identity, versions, and manifests

A package has a stable identity independent of its local directory name. Source projects declare identity, source inputs, and dependencies in a project manifest file named `project.json` at the project root. Source selection follows the manifest's normalized file and directory roots, include and exclude globs, and in-project containment rules. Compiled packages embed the relevant manifest data in the `.mfp` file.

Required manifest fields (validated by `validate_project_manifest`,
`src/main.rs`):

- `name`: the package import name used by source code.
- `version`: a semantic version `MAJOR.MINOR.PATCH`.
- `mfb`: the minimum compatible MFBASIC language version.
- `sources`: source files and roots selected by the project (each entry needs a
  `root`).
- `kind`: must be `package` for an imported package (`IMPORT_PACKAGE_KIND_INVALID`).

Optional string fields validated today: `entry`, `author`, `url`.

Dependency fields:

- `packages`: package dependency entries parsed by the resolver
  (`dependency_packages`, `src/resolver.rs`) with a `name` and an optional
  `source` locator.
- `native`: *(spec-level, not yet implemented)* native dependency metadata for
  packages that expose `LINK` bindings. The manifest parser does not currently
  read or validate a `native` key.

Per-dependency semantic-version *constraints* described below are part of the
intended package model; the resolver records each dependency's `name`/`source`
but the constraint-satisfaction and lockfile machinery is not exercised by the
current build path.

Version constraints use semantic-version ranges such as exact `=1.2.3`, compatible `^1.2.0`, patch-compatible `~1.2.0`, inequalities such as `>=1.2.0 <2.0.0`, or wildcard `1.2.*`. A dependency's selected version must satisfy every constraint that reaches it through the import graph.

The package resolver produces one selected version for each package identity. If two constraints cannot be satisfied by the same version, resolution fails with a package-version diagnostic; the compiler does not load multiple versions of the same package identity into one program.

A package may import a source package or an `.mfp` package. Imported `.mfp` packages must have a compatible binary representation/package format version, compatible public API metadata, and an MFBASIC language version supported by the compiler.

Executable builds are intended to use a lockfile named `mfb.lock`. The lockfile records the exact selected package identity, version, source or registry alias, content hash, binary representation/package version, native dependency metadata hash, and transitive dependencies. Locked builds must use the lockfile selections exactly; a hash or version mismatch fails before compilation, IR merging, or native linking. *(Implementers: the `LOCKFILE_MISMATCH` audit rule names this contract, but the current build/import-resolution path does not read or enforce an `mfb.lock`.)*

An **isolated function** is an exported top-level `FUNC` declared with `ISOLATED`. When an isolated function is used as a thread entry point, the runtime starts it in a fresh instance of its package. Starting isolated functions from the same package multiple times creates multiple independent instances; their top-level `MUT` bindings are not shared with each other or with the importing package.
