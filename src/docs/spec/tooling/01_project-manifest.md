# Project Manifest (project.json)

Every `mfb` project is rooted at a `project.json` manifest: a single top-level
JSON object that names the project, pins its toolchain and kind, declares its
source roots, and (optionally) its identity, signing metadata, and package
dependencies. The build pipeline loads and **validates** it before any
compilation; a malformed manifest aborts the build with a `PROJECT_JSON_*`
diagnostic. This topic owns the schema, the validation rules, and the diagnostic
codes; the commands that consume it are `./mfb spec architecture commands`.

## Top-Level Schema

```json
{
  "name": "myproject",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [
    { "root": "src", "role": "main", "include": ["**/*.mfb"] }
  ],
  "entry": "main",
  "targets": ["native"]
}
```

| field | type | required | meaning |
| --- | --- | --- | --- |
| `name` | string | yes | project name; non-empty after trim, and a single safe path component — `[A-Za-z0-9_][A-Za-z0-9_.-]*` — because every artifact path (`build/<name>.out`, `<name>.ast`, `build/<name>.app`, …) is formed by joining it (see `PROJECT_JSON_NAME_INVALID` below) |
| `version` | string | yes | project version; non-empty after trim. A macOS app build publishes it as the bundle's `CFBundleShortVersionString`/`CFBundleVersion` (./mfb spec linker macos-aarch64) |
| `mfb` | string | yes | toolchain/manifest schema version (`"1.0"`); non-empty after trim |
| `sources` | array of objects | yes | source roots (see *Source Entries*); non-empty |
| `kind` | string | yes¹ | `"executable"` or `"package"` |
| `mode` | string | no | `"console"` (default) or `"app"`; `"app"` is equivalent to passing `--app` (see ²) |
| `icon` | string | no | project-relative path to a 1024×1024 PNG source for the app icon, used on macOS and Linux (see ³) |
| `entry` | string | no | entry-point function name; defaults to `"main"` |
| `description` | string | yes⁵ | one-line summary shown on the registry; at most **4096 bytes**. Carried inside the signed `.mfp` payload as `PACKAGE_META` section 18 (./mfb spec package package-meta-section) |
| `author` | string | no | package author metadata |
| `url` | string | no | package homepage/source URL |
| `ident` | string | no | registry identity `<owner>#<package>`; a `--sign` build requires that shape (any owner prefix — after a transfer the recipient signs the transferred ident, and the registry decides who may) and defaults it to `<owner>#<name>` |
| `packages` | array of objects | no | declared dependencies (see *Dependency Entries*) |
| `libraries` | object | no | native `LINK` library locators, keyed by logical library name (see *Library Locator Entries*) |
| `resources` | array of objects | no | data files the build copies into the output tree (see *Resource Entries*) |
| `targets` | array | no | reserved; emitted by `mfb init` as `["native"]` and **not read by any stage** |
| `options` | object | no | runtime tunables baked into the executable (see ⁴) |
| `config` | object | no | named build configurations of String/Integer/Float values (see *Named configs*) |

Identity-chain fields (`identKey` and the key fingerprints) are **not**
manifest inputs: they are outputs of `mfb build --sign`, stamped into
the package metadata from the signing bundle. A manifest-level `identKey` is
ignored by the builder. (The per-dependency `identKey` *pin* inside
`packages[]` is different and load-bearing — see *Dependency Entries*.)

¹ `kind` is required by `validate_project_manifest`, but a present-and-string
value that is *neither* `executable` nor `package` only **warns**
(`PROJECT_JSON_UNKNOWN_KIND`) and validation continues; a missing or non-string
`kind` is a hard error. [[src/manifest/mod.rs:validate_kind]]

² `mode` composes with the `--app` CLI flag: app mode is requested if **either**
is set (`--app` is additive, never subtractive). Like `kind`, a present-and-string
`mode` that is neither `console` nor `app` only **warns**
(`PROJECT_JSON_UNKNOWN_MODE`) and continues; a non-string `mode` is a hard error.
App mode still requires `kind: "executable"` and an app-capable target (macOS/Linux).
[[src/manifest/mod.rs:validate_mode]] [[src/manifest/mod.rs:build_mode_is_app]]

³ `icon` is resolved and existence-checked only when app mode is active; a path
that does not resolve to a readable file is a hard error
(`PROJECT_JSON_ICON_MISSING`). The macOS backend renders it (or, when absent, the
compiler's embedded default) into `Contents/Resources/AppIcon.icns`; a provided
image that is not decodable or not exactly 1024×1024 fails the build. `icon` is
rendered on **both** platforms: a Linux app build does not ignore it, but writes
the PNG at every hicolor size into `usr/share/icons/hicolor/<N>x<N>/apps/` in the
AppDir plus a root copy. What is macOS-only is the `.icns` pipeline, not the key —
a malformed `icon` fails either app build.
[[src/os/linux/appdir.rs:HICOLOR_SIZES]]
[[src/manifest/mod.rs:icon_path]] [[src/os/macos/icon.rs:build_icns]]

The manifest validator checks required `name`/`version`/`mfb` strings,
optional `entry`/`author`/`url`/`icon` strings, `description`, `kind`, `mode`,
`sources`, `options.maxBuffer`, `resources`, `libraries`, `config`, and the array shape of
`packages`. Dependency-entry fields are interpreted by later package stages;
`ident` and `options` are read lazily. `targets` and per-source `role` are emitted
by `mfb init` but are not consulted by selection or build stages.
[[src/manifest/mod.rs:validate_project_manifest]]
[[src/manifest/mod.rs:validate_sources]] [[src/manifest/package.rs:package_metadata]]

⁴ `options` holds runtime tunables baked into the compiled executable.
Three keys are read: `canvasAutoPaintMS`, how long a `canvas::AutoPaint` holds its
enter and exit paints (below); `stdinLogCap`, the stdin broadcast-log
backpressure high-water mark in bytes — the runtime reader blocks a
producer rather than growing the log past `base + stdinLogCap`. It is read lazily
on the executable codegen path and defaults to `STDIN_LOG_CAP_DEFAULT` (4 MiB) when
absent, non-numeric, non-finite, or below one read chunk (8 KiB, which could not hold a single
chunk); and `maxBuffer`, the ceiling on one native `OUT CBuffer` allocation in MiB.
`maxBuffer` must be a whole number from 1 through 4096 and defaults to 64. Both
are read lazily on executable codegen and are fixed into the binary at build time;
they are not environment variables or runtime setters. Unknown keys are ignored.
`stdinLogCap` finite values are cast to `u64`: fractions truncate and values above
its range saturate. [[src/manifest/mod.rs:stdin_log_cap]]
[[src/manifest/mod.rs:max_buffer_bytes]]
The section was named `config` before plan-167; a manifest whose `config` object
still holds a `stdinLogCap` member fails validation with
`PROJECT_JSON_CONFIG_MOVED_OPTION` at that member, naming `options.stdinLogCap`.
There is no alias. [[src/manifest/mod.rs:validate_config]]

⁵ `description` is required for `kind: "package"` and optional for
`kind: "executable"`. When present on either kind, it must be a string of at
most 4096 bytes; an empty string is accepted. A package that omits it emits
`PROJECT_JSON_DESCRIPTION_MISSING`. Package builds carry it in the signed
`PACKAGE_META` payload; see `./mfb spec package package-meta-section`.
[[src/manifest/mod.rs:MAX_DESCRIPTION_BYTES]]
[[src/manifest/mod.rs:validate_description]]

## AutoPaint flash length (`options.canvasAutoPaintMS`)

A `canvas::AutoPaint` shows its `enterPaint` for a fixed time after the pointer arrives
on it and its `exitPaint` for the same time after the pointer leaves
(`./mfb spec app canvas`, "AutoPaint"). `options.canvasAutoPaintMS` is that time, in
milliseconds:

```json
{ "options": { "canvasAutoPaintMS": 100 } }
```

- Whole number, 1 to 10000. Anything else — a string, a fraction, zero, out of range —
  is `PROJECT_JSON_FIELD_TYPE`. [[src/manifest/mod.rs:validate_canvas_auto_paint_ms]]
- Omitted, it defaults to **100**. `mfb init` and `mfb init-pkg` write the default out.
  [[src/manifest/mod.rs:DEFAULT_CANVAS_AUTO_PAINT_MS]]
- It is the **consuming executable's** setting, like `maxBuffer`: a package that draws
  an `AutoPaint` gets the timing of the program that imports it, and a package's own
  value is not encoded into its `.mfp`. [[src/manifest/mod.rs:canvas_auto_paint_ms]]
- It is one value for the whole program. An `AutoPaint` has no field for it.

The build bakes the value into the last word of the canvas graphics-state block, the one
word of that block that does not start as zero
([[src/codegen/runtime/canvas/mod.rs:GRAPHICS_OFFSET_AUTO_WINDOW_MS]]), and the canvas
companion reads it with `canvas::autoWindowMs`. A program that does not use `canvas` has
no such block and the key has no effect.

## Native buffer ceiling (`options.maxBuffer`)

A native `LINK` wrapper with an `OUT CBuffer` slot sizes its buffer from a `BUFFER … SIZE` expression over the wrapper's parameters (./mfb spec language native-libraries). That size is a *runtime* value chosen by the caller, so without a ceiling it is an unbounded allocation request.

`options.maxBuffer` sets that ceiling, in MiB:

```json
{ "options": { "maxBuffer": 256 } }
```

- Whole number, 1 to 4096. Anything else — a string, a fraction, zero, out of range — is `PROJECT_JSON_FIELD_TYPE`.
- Omitted, it defaults to **64** MiB, which is about 5.8 minutes of stereo 48 kHz 16-bit audio.
- A `SIZE` above the ceiling (or negative) raises `ErrInvalidArgument` **before** allocating, so an over-large request costs nothing.

It is the **consuming** project's setting. `LINK` thunks are emitted when an executable links, so the application importing a binding decides how much memory one native read may claim; a binding package neither carries a ceiling nor can raise the application's.

## Named configs (`config`)

`config` declares named build configurations. Each named config is a flat object
of keys to String, Integer or Float values:

```json
"config": {
  "release": { "apiUrl": "https://api.example.com", "retries": 3, "timeout": 2.5 },
  "dev":     { "apiUrl": "http://localhost:8080" }
}
```

The section is read with a lossless JSON reader rather than the parser used for
the rest of the manifest, because these rules depend on what that parser drops:
a number's spelling, repeated keys, and the line of each key. Every rule below is
an error, all of them are reported in one run, and each is reported at the line
of the offending key. [[src/manifest/config.rs:parse_config]]
[[src/json/lossless.rs:parse_lossless]]

- **Optional.** An absent `config` is one implicit, empty `release` config.
- **Shape.** `config` must be an object, and each member's value must be an
  object (`PROJECT_JSON_CONFIG_SHAPE`).
- **`release` is required.** A present `config` must have a member named exactly
  `release` (`PROJECT_JSON_CONFIG_RELEASE_MISSING`, at the `config` line). The
  check is skipped when `config` holds a `stdinLogCap` member, which reports
  `PROJECT_JSON_CONFIG_MOVED_OPTION` instead, so a pre-plan-167 manifest gets one
  diagnostic.
- **Names.** Config names and keys match `[A-Za-z][A-Za-z0-9_.-]*`
  (`PROJECT_JSON_CONFIG_NAME`). A dotted key such as `db.host` is one key, not a
  path.
- **Duplicates.** Two config names, or two keys of one config, that are equal under
  ASCII case-folding are an error at the later one (`PROJECT_JSON_CONFIG_DUPLICATE`),
  as is a repeated top-level `config`. Lookup is still exact: only one spelling of a
  name can be declared.
- **Values** (`PROJECT_JSON_CONFIG_VALUE`). A JSON string is a String; it may not
  contain U+0000. A number with no `.`, `e` or `E` is an Integer and must fit `i64`
  (`9223372036854775808` is an error, not a Float). A number with a `.` or an
  exponent is a Float, correctly rounded, and must be finite (`1e999` is an
  error). `-0` is the Integer 0 and `0.0` is a Float. `true`, `false`, `null`,
  arrays and objects are errors.
- **One type per key** (`PROJECT_JSON_CONFIG_TYPE_CONFLICT`). A key declared in
  more than one named config has the same type in all of them, so a program cannot
  build under one config and fail to build under another. The conflict is
  reported at the later declaration, naming the first config that declared the key.
  [[src/manifest/config.rs:check_type_consistency]]

A build selects one named config: the one `--config <name>` names, or `release`
(`./mfb spec tooling cli-reference`). A name `config` does not declare is
`PROJECT_JSON_CONFIG_UNKNOWN` at the `config` line (line 1 when the section is
absent, which declares only the implicit `release`), and the detail lists the
declared names. [[src/manifest/config.rs:ProjectConfig]]

These checks run wherever the manifest is validated: `mfb build`, `mfb test`,
`mfb fmt`, `mfb doc`, `mfb pkg`, and the build of each source dependency inside a
consuming project's build. A package's `config` is validated that way, but its
values never apply to the consuming program. [[src/manifest/mod.rs:validate_config]]

## Local overrides (`project-local.json`)

An optional `project-local.json` beside `project.json` overrides and extends the
`config` section for one machine: API keys, passwords, and local developer
settings. It is meant to stay out of source control, and the `.gitignore` that
`mfb init` and `mfb init-pkg` write names it.
[[src/manifest/config.rs:load_project_config]]

```json
{ "config": { "dev": { "apiUrl": "http://localhost:9000", "apiKey": "secret" } } }
```

- **Scope.** The root must be an object whose only member is an optional
  `config`. Any other member, or a root that is not an object, is
  `PROJECT_LOCAL_JSON_FIELD`. The file cannot change `name`, `packages`,
  `options` or anything else that feeds the lockfile, `projectHash`, or a
  `mfb pkg` rewrite, none of which ever read it.
- **Validation.** Its `config` follows every *Named configs* rule except that
  `release` is not required, since the file is partial. A malformed file is
  `PROJECT_JSON_PARSE_FAILED`. Every diagnostic names `project-local.json` and
  the line in it.
- **Merge.** For each named config in the local file: if `project.json` declares
  a config with the same name, each local key replaces the same key there or is
  added to it. A replacement must keep the key's type
  (`PROJECT_LOCAL_JSON_TYPE_OVERRIDE`, at the local line, naming the
  `project.json` line). A config `project.json` does not declare is added as a
  new named config, selectable with `--config` on that machine. Overriding is by
  exact name, so a local config name or key that matches a `project.json` one only
  when case is ignored is `PROJECT_JSON_CONFIG_DUPLICATE`, because it would
  declare a second spelling. Duplicates within the local file are checked the same
  way as in `project.json`.
- **After the merge** the one-type-per-key rule runs over the union, so a locally
  added key that conflicts with another config's type is
  `PROJECT_JSON_CONFIG_TYPE_CONFLICT` in the local file. Selection
  (`--config`, `PROJECT_JSON_CONFIG_UNKNOWN`) happens on the merged result.
- **Who reads it.** Only the build of the invoked project (`mfb build`,
  `mfb test`, and `mfb pkg`'s publish and ABI-check builds), on every path,
  executable or package. `validate_project_manifest` checks `project.json` alone,
  so `mfb fmt` and `mfb doc` never read the local file, and a source dependency's
  local file is never read, even when the dependency is a local directory that
  has one. [[src/cli/build/mod.rs:select_build_config]]

## Source Entries

Each element of `sources[]` is an object describing one source root:

| field | type | required | meaning |
| --- | --- | --- | --- |
| `root` | string | yes | directory or `.mfb` file (relative to the project) to select; non-empty after trim |
| `include` | array of strings | no | glob patterns to include |
| `exclude` | array of strings | no | glob patterns to exclude |
| `role` | string | no | reserved role tag (`"main"`/`"package"`); neither validated nor read |

`root` is validated as a required, non-empty string. `include` and `exclude`,
when present, must each be an **array of strings** — any non-array value, or an
array containing a non-string element, is a `PROJECT_JSON_FIELD_TYPE` error.
A `sources[]` element that is not a JSON object is a `PROJECT_JSON_FIELD_TYPE`
error reported as `Source entry #N must be an object.`. [[src/manifest/mod.rs:validate_sources]] [[src/manifest/mod.rs:validate_source_pattern_field]]

The glob algorithm that turns these patterns into the `.mfb` input set is
`./mfb spec tooling source-selection`. The selector does not read `role` — every source root is treated the
same regardless of its `role` tag.

## Dependency Entries

Each element of `packages[]` declares one dependency:

| field | type | default | meaning |
| --- | --- | --- | --- |
| `name` | string | — (required) | dependency package name; blank → entry ignored |
| `ident` | string | falls back to `name` | publisher identity of the dependency |
| `version` | string | `""` | requested version |
| `pin` | bool | `false` | when true, the installed `.mfp` must match `version` exactly |
| `source` | string | `""` | origin URL the dependency was added from |
| `identKey` | string | `""` | the pinned ident public key — the trust anchor: the verified `identKey` of the installed release. Written by `pkg add` on first add of a signed package (trust-on-first-use, verified for the release's publisher through the registry's signed transfer chain); every later build verifies the installed `.mfp` against this pin, never against the file-embedded key. `pkg update`/`pkg verify` move it when the pin reaches a new release's key through the owner's rotation chain or a signed ownership transfer, with a notice. Snake_case `ident_key` is accepted on read. |
| `direct` | bool | none | true when the user declared this dependency themselves; false when `mfb pkg` added it because another declared package's import table names it. Absent on an entry the closure rule below flags as inconsistent. |
| `requiredBy` | array of strings | none | the idents of the declared packages whose import tables name this dependency; empty for a package nothing else requires. Order-insensitive; `mfb pkg` writes it sorted. Absent has the same inconsistency treatment as a missing `direct`. |

`packages` must be an array when present (`validate_packages_array`); each
element must be an object with a string `name`. An entry whose `name` is absent,
non-string, or blank-after-trim is silently skipped. `pin = true` makes the
build reject an installed `packages/<name>.mfp` whose header version differs from
the declared `version`. [[src/manifest/package.rs:project_package_dependency]] [[src/manifest/package.rs:package_dependencies]] [[src/manifest/package.rs:installed_package_files]]

The registry/add workflow that writes these entries is
`./mfb spec package-manager`; the on-disk `.mfp` header they resolve
against is `./mfb spec package container-format`.

### The declared dependency closure

A native build merges exactly the packages `project.json` declares — nothing
more is pulled in implicitly — so `packages[]` must list a project's **whole
dependency closure**: every package that any declared package's own import
table names must also be declared here, not only the packages the project's
own source imports. This is a declaration/merging rule, not a visibility one:
imports stay non-transitive for *names* — a project still cannot write
`basepkg::` without its own `IMPORT basepkg` (`./mfb spec language
modules-and-packages`).
[[src/manifest/package.rs:installed_package_files]] [[src/target/shared/nir/lower.rs:merge_packages]]

What a declared package "imports" is read the same way the build reads it,
in this order: an installed `packages/<name>.mfp`'s import table wins; else,
for a source-directory dependency, that directory's own `project.json`
`packages[]` (exactly what the package build writes into its import table);
else the build cache `build/packages/<name>.mfp`. A payload that cannot be
decoded contributes nothing to the requirement graph — verification and merge
report the broken file far more precisely.
[[src/manifest/closure.rs:requirements]] [[src/manifest/closure.rs:import_table_requirements]] [[src/manifest/closure.rs:source_manifest_requirements]]

`mfb build` never repairs `project.json`; it only refuses an inconsistent one
before compiling (see *Diagnostic Codes* below), located at the `packages`
field. [[src/manifest/closure.rs:check]] [[src/manifest/closure.rs:conflicts]]
`mfb pkg` is the single path that writes the closure — `add`/`update`/`remove`
all route through `apply_manifest_change`, which closes the proposed manifest
before it resolves or writes `project.json`. [[src/manifest/closure.rs:reconcile]]
See `./mfb spec tooling cli-reference` for the `pkg` behavior.

An entry is dropped only when its `direct` is `false` **and** its
`requiredBy` is empty; a `direct: true` entry — one the user wrote — is never
dropped just because nothing currently imports it.

`projectHash` (the value `mfb.lock` is keyed against) covers only
`name`/`ident`/`version`/`pin`/`source`, so `direct` and `requiredBy` changing
never invalidates a lock. [[src/audit/collect/mod.rs:project_hash]]

Example: an app that declares `userpkg` (which itself imports `basepkg`)
declares both, the second as an indirect closure entry:

```json
"packages": [
  { "name": "userpkg", "version": "1.0.0", "source": "file:../userpkg",
    "direct": true, "requiredBy": [] },
  { "name": "basepkg", "version": "1.0.0", "source": "file:../basepkg",
    "direct": false, "requiredBy": ["userpkg"] }
]
```

## Library Locator Entries

`libraries` maps each native `LINK` **logical library name** (the string in
`LINK "sqlite3"`) to an ordered array of **locators**, each naming the concrete
shared object to load for one slice of the target matrix:

```json
"libraries": {
  "sqlite3": [
    { "os": "macos", "type": "system", "source": "libsqlite3.dylib" },
    { "os": "linux", "type": "system", "source": "libsqlite3.so.0" },
    { "os": "linux", "arch": "riscv64", "libc": "musl", "source": "libsqlite3-riscv64-musl.so" }
  ]
}
```

Read as: use the system `libsqlite3.dylib` on macOS; use the system
`libsqlite3.so.0` everywhere on Linux — any arch, either libc — **except** on
riscv64/musl, where the third entry wins on specificity and loads a vendored file
from `<project root>/vendor/`.

| field | type | default | meaning |
| --- | --- | --- | --- |
| `os` | string | — (required) | `macos`, `linux`, or `windows`; the canonical `BuildTarget.os` token of a registered backend |
| `arch` | string | *any arch* | `aarch64`, `x86_64`, or `riscv64`; omitted = wildcard |
| `libc` | string | *any libc* | `glibc` or `musl`; omitted = wildcard. Linux only — macOS has no libc axis |
| `type` | string | `vendor` | `system` (found by the dynamic loader) or `vendor` (a file shipped in `vendor/`) |
| `source` | string | — (required) | a **bare filename**, never a path |

The value set for `os`/`arch` is read from the native backend registry, so
registering a backend widens the accepted vocabulary automatically.
[[src/target.rs:registered_target_oses]] [[src/target.rs:registered_target_arches]]
[[src/manifest/libraries.rs:project_libraries]]

### `type` defaults to `vendor`, deliberately

An absent `type` means `vendor` because that **fails closed**. A missing or
typo'd `type` under a `vendor` default resolves to `<root>/vendor/<source>` and
hard-errors at build time when the file is absent. Under a `system` default, the
same mistake would silently hand `source` to the dynamic loader, which would
search the system library path and load whatever it found under that name — a
wrong-library load that only surfaces at runtime, and a supply-chain footgun. A
build error beats a silent mis-resolution.

### `system` may wildcard; `vendor` must name its exact target

- **`system`** means *"ask the loader for this name"* — the platform supplies the
  build that fits, so `arch` and `libc` are legitimately omittable. One
  `{ "os": "linux", "type": "system", "source": "libsqlite3.so.0" }` covers all
  six Linux slots.
- **`vendor`** means *"load this exact file I shipped"* — one concrete artifact
  compiled for exactly one `(os, arch, libc)` triple. A `.so` built against glibc
  will not load on musl; an x86_64 `.so` will not load on aarch64. **There is no
  fat ELF.**

So a `vendor` locator on `os: "linux"` **must** specify both `arch` and `libc`;
omitting either is `PROJECT_JSON_LIBRARY_INVALID`. macOS is exempt from the
`arch` half — Mach-O fat binaries are real, so a universal `.dylib` with `arch`
omitted is a legitimate locator.

Because a `vendor` locator always carries concrete axes, it always outranks a
wildcarding `system` locator on specificity — *"use my vendored build on musl,
the system library everywhere else"* needs no special rule.

### `source` is a bare filename

`source` names a file, never a location. It is rejected when it is blank,
contains a path separator (`/` or `\`), is `.` or `..`, carries a Windows drive
prefix (`C:`), or contains a NUL byte (it is emitted verbatim as a C string into
the binary, so an interior NUL would silently truncate the library name).

For a `system` locator, `source` is the exact soname handed to the dynamic
loader. For a `vendor` locator, the file must live at
**`<project root>/vendor/<source>`** — flat, no subdirectories. The resolved path
is never spelled in the manifest; it is always `vendor/` + `source`.

Because `vendor/` is flat, **one filename means one file**: two `vendor` locators
sharing a `source` anywhere in the section — across *all* logical names — is
`PROJECT_JSON_LIBRARY_SOURCE_CONFLICT`. The check is scoped to `vendor` locators only: `system` sonames legitimately repeat,
and `linux/x86_64` and `linux/aarch64` both asking for `libsqlite3.so.0` is the
normal case. [[src/manifest/mod.rs:validate_libraries]]

The section is validated for *shape* only — it is not checked against the
filesystem here. Whether a vendored file exists and what it hashes to is resolved
when the package is built; see `./mfb spec language native-libraries`.

## Resource Entries

`resources` is an array of `{src, dst}` objects naming data files the build copies
into the output tree beside the executable. Both fields are required
strings and both are shape-validated at manifest load, not deferred.

| field | type | required | meaning |
| --- | --- | --- | --- |
| `src` | string | yes | project-relative glob of files to copy; non-empty |
| `dst` | string | yes | destination path within the output tree; a trailing `/` names a directory |

The validator checks path components textually: `src` must not escape the
project root and `dst` must not escape the output tree. `copy_resources`
additionally canonicalizes source paths at copy time, rejecting a symlink
inside the project that points outside it.
[[src/manifest/mod.rs:validate_resources]] [[src/cli/build/resources.rs:copy_resources]]

At runtime a program locates a copied resource with `os::appResourcePath` — see
`./mfb spec stdlib os`.

## Entry Point Validation

For `kind = "package"` no entry point is required and validation is skipped.
For `kind = "executable"` the manifest's `entry` (default `"main"`) must resolve
to exactly **one** top-level function across all source files, subject to these
rules — each failure emits `PROJECT_ENTRY_INVALID`: [[src/manifest/entry.rs:validate_entry_point]]

| condition | message gist |
| --- | --- |
| no function named `entry` | must declare an entry point named `<entry>` |
| more than one matching function | must declare exactly one entry point |
| `FUNC` entry whose return type is neither `Integer` nor `Nothing` | entry `FUNC` must return Integer or Nothing |
| a single parameter whose type ≠ `List OF String` | parameter must have type `List OF String` |
| more than one parameter | must declare zero params or one `args AS List OF String` |
| the `args` parameter declares a default value | args parameter must not declare a default |

The entry `FUNC` must return `Integer` or `Nothing`; the native entry wrapper
branches on `returns` alone.
[[src/codegen/engine/function/entry.rs:lower_program_entry]] The accepted parameter shapes are exactly: zero parameters, or
one `args AS List OF String` parameter (no default). The resolved entry becomes
`ir::EntryPoint { name, returns, accepts_args }`. [[src/manifest/entry.rs:validate_entry_point]]

## Loading and Parse Errors

`validate_project_manifest` loads the file and walks the schema in order,
emitting these structural diagnostics before any per-field check: [[src/manifest/mod.rs:validate_project_manifest]]

| stage | code | trigger |
| --- | --- | --- |
| existence | `PROJECT_JSON_MISSING` | the `project.json` path does not exist |
| read | `PROJECT_JSON_READ_FAILED` | the file exists but cannot be read |
| parse | `PROJECT_JSON_PARSE_FAILED` | contents are not valid JSON (carries line/column) |
| root type | `PROJECT_JSON_ROOT_TYPE` | the top-level JSON value is not an object |

The first three abort immediately; field validation only runs once the manifest
parses to an object. After the structural pass, required-string, sources,
optional-string, and kind checks each accumulate into a `valid` flag — all field
errors in a single run are reported, then validation returns `Err` if any
failed. [[src/manifest/mod.rs:validate_project_manifest]]

## Diagnostic Codes

All manifest and entry-point diagnostics live in the `2-200-####` rule range
(see `./mfb spec diagnostics rule-codes`):

| code | name | severity | trigger |
| --- | --- | --- | --- |
| `2-200-0001` | `PROJECT_JSON_MISSING` | error | `project.json` does not exist [[src/manifest/mod.rs:validate_project_manifest]] |
| `2-200-0002` | `PROJECT_JSON_READ_FAILED` | error | file present but unreadable |
| `2-200-0003` | `PROJECT_JSON_PARSE_FAILED` | error | not valid JSON |
| `2-200-0004` | `PROJECT_JSON_ROOT_TYPE` | error | top-level value is not an object |
| `2-200-0005` | `PROJECT_JSON_REQUIRED_FIELD` | error | a required field (`name`/`version`/`mfb`/`sources`/`kind`/source `root`) is missing [[src/manifest/mod.rs:validate_required_string]] |
| `2-200-0006` | `PROJECT_JSON_FIELD_TYPE` | error | a field has the wrong type (non-string scalar, non-array `sources`/`include`/`exclude`, non-object source entry) [[src/manifest/mod.rs:validate_optional_string]] |
| `2-200-0007` | `PROJECT_JSON_EMPTY_FIELD` | error | a required string is empty after trim [[src/manifest/mod.rs:validate_required_string]] |
| `2-200-0008` | `PROJECT_JSON_EMPTY_SOURCES` | error | `sources` is present but an empty array [[src/manifest/mod.rs:validate_sources]] |
| `2-200-0009` | `PROJECT_JSON_UNKNOWN_KIND` | warn | `kind` is a string other than `executable`/`package` (non-fatal) [[src/manifest/mod.rs:validate_kind]] |
| `2-200-0010` | `PROJECT_JSON_VALID` | info | reserved; not emitted — a successful validation is silent |
| `2-200-0011` | `PROJECT_ENTRY_INVALID` | error | executable entry-point resolution failed [[src/manifest/entry.rs:validate_entry_point]] |
| `2-200-0012` | `PROJECT_JSON_UNKNOWN_MODE` | warn | string `mode` is neither `console` nor `app` [[src/manifest/mod.rs:validate_mode]] |
| `2-200-0013` | `PROJECT_JSON_ICON_MISSING` | error | app icon path does not resolve to a file [[src/manifest/mod.rs:icon_path]] |
| `2-200-0014` | `PROJECT_JSON_LIBRARY_INVALID` | error | a `libraries` locator is malformed, carries an unknown `os`/`arch`/`libc`/`type` token, sets `libc` on macOS, omits `arch`/`libc` on a Linux `vendor` entry, or names a `source` that is not a bare filename [[src/manifest/mod.rs:validate_libraries]] |
| `2-200-0015` | `PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` | error | two `vendor` locators declare the same `source` filename (see *Library Locator Entries*) [[src/manifest/mod.rs:validate_libraries]] |
| `2-200-0016` | `PROJECT_JSON_DESCRIPTION_MISSING` | error | `kind` is `package` and no `description` is declared [[src/manifest/mod.rs:validate_description]] |
| `2-200-0017` | `PROJECT_JSON_NAME_INVALID` | error | `name` is not a single safe path component (`[A-Za-z0-9_][A-Za-z0-9_.-]*`): it contains a path separator, starts with `.`, or is `..`. Every artifact writer joins the name onto a directory, so a traversing name would place a `0755` executable outside the project [[src/manifest/mod.rs:validate_name]] |
| `2-200-0018` | `PROJECT_JSON_CONFIG_MOVED_OPTION` | error | the `config` object holds a `stdinLogCap` member; the tunable moved to `options.stdinLogCap` [[src/manifest/mod.rs:validate_config]] |
| `2-200-0019` | `PROJECT_JSON_CONFIG_SHAPE` | error | `config` is not an object, or a named config's value is not an object (see *Named configs*) [[src/manifest/config.rs:parse_config]] |
| `2-200-0020` | `PROJECT_JSON_CONFIG_RELEASE_MISSING` | error | `config` is present without a `release` member |
| `2-200-0021` | `PROJECT_JSON_CONFIG_NAME` | error | a config name or key does not match `[A-Za-z][A-Za-z0-9_.-]*` |
| `2-200-0022` | `PROJECT_JSON_CONFIG_DUPLICATE` | error | a config name, a key within one config, or the top-level `config` is declared twice, compared case-insensitively |
| `2-200-0023` | `PROJECT_JSON_CONFIG_VALUE` | error | a config value is not a String, an `i64` Integer, or a finite Float, or is a String containing U+0000 |
| `2-200-0024` | `PROJECT_JSON_CONFIG_TYPE_CONFLICT` | error | a key has different types in two named configs [[src/manifest/config.rs:check_type_consistency]] |
| `2-200-0025` | `PROJECT_JSON_CONFIG_UNKNOWN` | error | the config `--config` names (or `release`) is not declared [[src/cli/build/mod.rs:select_build_config]] |
| `2-200-0026` | `PROJECT_LOCAL_JSON_FIELD` | error | `project-local.json` is not an object, or holds a member other than `config` (see *Local overrides*) [[src/manifest/config.rs:load_project_config]] |
| `2-200-0027` | `PROJECT_LOCAL_JSON_TYPE_OVERRIDE` | error | `project-local.json` overrides a config value with a different type |

The two dependency-closure codes below live in the `6-605-####` package range
(see `./mfb spec diagnostics rule-codes`), not `2-200-####`, because they are
raised by the build's package-verification stage rather than manifest-shape
validation:

| code | name | severity | trigger |
| --- | --- | --- | --- |
| `6-605-0013` | `PACKAGE_DEPENDENCIES_INCONSISTENT` | error | `packages[]` disagrees with the declared packages' import tables: a needed package is not declared; a needed package's name is declared under a different `ident`; an entry is missing a boolean `direct`; an entry is missing a `requiredBy` array of strings; a `requiredBy` differs from what the import tables require; or an entry has `direct: false` and nothing requires it [[src/manifest/closure.rs:check]] [[src/cli/build/packages.rs:refuse_inconsistent_closure]] |
| `6-605-0014` | `PACKAGE_VERSION_CONFLICT` | error | reported only once `6-605-0013` finds nothing: a declared importer's import table records a used symbol whose ABI hash the declared dependency's compiled `.mfp` does not export with that hash, or the importer pins an exact version the installed dependency's manifest version is not [[src/manifest/closure.rs:conflicts]] [[src/cli/build/packages.rs:refuse_inconsistent_closure]] |

Both are checked by `verify_and_report_packages` after per-package trust
verification and only when no package was refused on trust grounds, and both
stop the build before compiling the project; the message for each names the
follow-up command (`mfb pkg update` to rewrite `project.json`, `mfb pkg
verify` for per-symbol detail).
[[src/cli/build/packages.rs:verify_and_report_packages]]

`PROJECT_JSON_LIBRARY_INVALID` covers a dozen distinct mistakes, so its
**message** — not just its code — names the specific cause: which field, which
token, and the accepted set.

`PROJECT_JSON_REQUIRED_FIELD` is reused for a missing source `root` (recursing
through `validate_required_string` on the source object), and
`PROJECT_JSON_EMPTY_FIELD` for a blank one. Diagnostic spans point at the
offending field's position in the raw source text (or a fallback position when
the field is absent). [[src/manifest/mod.rs:validate_sources]]

## See Also

* ./mfb spec architecture commands — the build commands that load and act on this manifest
* ./mfb spec architecture packages — how dependency manifests feed the package build path
* ./mfb spec tooling source-selection — the glob algorithm over `sources[]`
* ./mfb spec diagnostics rule-codes — the full `2-200-####` diagnostic catalogue
* ./mfb spec package container-format — the `.mfp` header that `pin`/`version` resolve against
* ./mfb spec package-manager — the `pkg add`/publish workflow that writes `packages[]`
