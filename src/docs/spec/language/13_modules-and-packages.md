# 13. Modules & Packages

**Project source = one package.** The `.mfb` files selected by the current
project's `project.json` together form that project's source package.
Directories inside a source root do not create package boundaries or package
namespaces. Additional packages are introduced only through the importing
project's `project.json` `packages` array.

Visibility (default is `PUBLIC`): [[src/ast/types.rs:Visibility]]
- `PRIVATE` — file-local (opt in explicitly). A `PRIVATE` top-level declaration is
  scoped to its own file: two files may each declare a `PRIVATE` symbol of the same
  name without colliding (the compiler renames each to a file-unique internal name
  before resolution). Where a file's own `PRIVATE` declaration shares a name with a
  project `PUBLIC` declaration, the `PRIVATE` one wins *within that file* and the
  compiler emits a `PRIVATE_SHADOWS_PUBLIC` warning.
- `PUBLIC` (default) — visible to all files in the same package, hidden from importers.
- `EXPORT` — visible to importers. `EXPORT` is the flag that writes a symbol into
  the compiled `.mfp` public API, so it is **valid only in a `kind: "package"`
  project**; a top-level `EXPORT` in an executable is rejected with
  `EXPORT_IN_EXECUTABLE` (use `PUBLIC` — the default — for project-wide visibility
  in an executable).

Within a single build, `PUBLIC` and `EXPORT` are treated **identically** by
the visibility check: [[src/resolver/mod.rs:visible_from]] [[src/syntaxcheck/mod.rs:visible_from]] both are visible across
files in the project, and only `PRIVATE` is file-local. The `PUBLIC`/`EXPORT`
distinction matters only for what is written into the compiled `.mfp` package
(the exported-symbol flag), not for in-project name resolution. This is why a
cross-file reference to a `PRIVATE` declaration fails unless the declaration is
`PUBLIC`/`EXPORT` or the project is built as a single file.

Top-level `LET`, `MUT`, `FUNC`, `SUB`, `TYPE`, `UNION`, `ENUM`, and `RESOURCE` may use `PRIVATE`, `PUBLIC`, or `EXPORT`. [[src/ir/lower_link.rs:native_resources]] (`RESOURCE` was missing from this list, which led bug-288 to propose rejecting `PRIVATE RESOURCE` outright even though resource visibility is modelled and lowered; a `PRIVATE` resource is file-local exactly as a `PRIVATE TYPE` is.) Fields in `TYPE` declarations may also use `PRIVATE`, `PUBLIC`, or `EXPORT`; omitted field visibility defaults to `EXPORT` when the containing type is `EXPORT`, otherwise to `PUBLIC` — i.e. the containing type's visibility, capped at `PUBLIC` for non-exported types. [[src/syntaxcheck/helpers.rs:effective_field_visibility]]

Only project-visible top-level `FUNC` declarations may use `ISOLATED` — i.e.
`PUBLIC` (the default) or `EXPORT`, not `PRIVATE`. Imported package constructors are addressed as `package::identifier` when constructing values, but constructors for records with hidden fields are callable only from scopes that can see every required field.

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
- An import alias must not conflict with another imported package name or alias, a top-level declaration visible in the file, or a built-in package name. The built-in packages are: `bits`, `collections`, `crypto`, `csv`, `datetime`, `encoding`, `errorCode`, `fs`, `http`, `io`, `json`, `math`, `net`, `os`, `regex`, `strings`, `term`, `thread`, `tls`, and `vector`. [[src/builtins/mod.rs:is_builtin_import]]
- `[visibility] FUNC alias AS qualified::name` declares a **function alias**. This form exists only as a transparent re-export of a native `LINK` function: the resolver requires the alias target to resolve to a `LINK` (native) function signature and reports `SYMBOL_UNKNOWN_IDENTIFIER` otherwise. [[src/resolver/mod.rs:link_target_signature]] It is not a general mechanism for aliasing arbitrary MFBASIC functions to package-qualified names.

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
names [[src/resolver/packages.rs:install_package_type_names]] and never re-processes
the dependency's own imports, so transitivity does not occur structurally. (Note
to implementers: the compiler does **not** currently detect or diagnose an
import *cycle* — there is no circular-dependency check on the import graph;
recursive-record cycle detection is unrelated.)

`IMPORT packageName` resolves a package, not an arbitrary source file. The
compiler resolves the first identifier in the import using this order: [[src/resolver/packages.rs:resolve_imported_package]]

1. A built-in package supplied by the toolchain, such as `io`;
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
[[src/resolver/packages.rs:resolve_imported_package]]

`<project_root>/packages` is the resolved dependency store, similar in role to
`node_modules` in Node projects. It is managed by the package manager. The
compiler does not implicitly import undeclared packages from this directory.
Each package is responsible for declaring its own dependencies; dependency
declarations are not inherited from importers and imports are not transitive.

## 13.1 Package identity, versions, and manifests

A package has a stable identity independent of its local directory name. Source
projects declare identity, source inputs, and dependencies in a project manifest
file named `project.json` at the project root. The required fields a source file
sees are `name` (the import name used by source), `version` (a semantic version
`MAJOR.MINOR.PATCH`), `mfb` (the minimum compatible language version), `sources`
(the selected file/directory roots), and `kind` (`package` for an imported
package). Compiled packages embed the relevant manifest data in the `.mfp` file.

The full manifest field set, dependency entries, semantic-version constraint
ranges, lockfile (`mfb.lock`) policy, and how the package manager installs and
pins dependencies are tooling concerns documented by `./mfb spec architecture
packages`. The on-disk byte encoding of identity/version/entry into the `.mfp`
`MANIFEST` section is owned by `./mfb spec package metadata-encoding`. A package
may import a source package or an `.mfp` package; imported `.mfp` packages must
carry a compatible package-format version, compatible public API metadata, and a
language version the compiler supports.

An **isolated function** is an exported top-level `FUNC` declared with `ISOLATED`. When an isolated function is used as a thread entry point, the runtime starts it in a fresh instance of its package. Starting isolated functions from the same package multiple times creates multiple independent instances; their top-level `MUT` bindings are not shared with each other or with the importing package.

## See Also

* ./mfb spec architecture packages — manifest fields, version constraints, lockfile, install/pin policy
* ./mfb spec package metadata-encoding — `.mfp` `MANIFEST` byte encoding
* ./mfb spec language threads — isolated-function thread entry points
