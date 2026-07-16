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
| `name` | string | yes | project name; non-empty after trim |
| `version` | string | yes | project version; non-empty after trim. A macOS app build publishes it as the bundle's `CFBundleShortVersionString`/`CFBundleVersion` (./mfb spec linker macos-aarch64) |
| `mfb` | string | yes | toolchain/manifest schema version (`"1.0"`); non-empty after trim |
| `sources` | array of objects | yes | source roots (see *Source Entries*); non-empty |
| `kind` | string | yes¹ | `"executable"` or `"package"` |
| `mode` | string | no | `"console"` (default) or `"app"`; `"app"` is equivalent to passing `--app` (see ²) |
| `icon` | string | no | project-relative path to a 1024×1024 PNG source for the macOS app icon (see ³) |
| `entry` | string | no | entry-point function name; defaults to `"main"` |
| `author` | string | no | package author metadata |
| `url` | string | no | package homepage/source URL |
| `ident` | string | no | registry identity `<owner>#<package>`; a `--sign` build requires it to belong to the signing owner and defaults it to `<owner>#<name>` |
| `packages` | array of objects | no | declared dependencies (see *Dependency Entries*) |
| `targets` | array | no | build targets; emitted by `mfb init` as `["native"]` |
| `config` | object | no | build-time runtime tunables baked into the executable (see ⁴) |

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
macOS-only — a Linux/GTK app build ignores it.
[[src/manifest/mod.rs:icon_path]] [[src/os/macos/icon.rs:build_icns]]

Only `name`/`version`/`mfb` (required strings), `entry`/`author`/`url`/`icon`
(optional strings), `kind`, `mode`, and `sources` are *validated* by the manifest
validator. The
remaining fields (`ident`, `packages`, `targets`, `config`, and the per-source
`role`) are read lazily by later stages — `package_metadata`,
`package_dependencies`, the source selector, and the codegen tunable readers — and
are **not** schema-checked here; an absent or wrong-typed value simply defaults
rather than erroring.
[[src/manifest/mod.rs:validate_project_manifest]] [[src/manifest/package.rs:package_metadata]]

⁴ `config` holds build-time runtime tunables baked into the compiled executable
(plan-15 D3). Currently one key is read: `stdinLogCap`, the stdin broadcast-log
backpressure high-water mark in bytes (plan-15 §4.1) — the runtime reader blocks a
producer rather than growing the log past `base + stdinLogCap`. It is read lazily
on the executable codegen path and defaults to `STDIN_LOG_CAP_DEFAULT` (4 MiB) when
absent, non-numeric, or below one read chunk (8 KiB, which could not hold a single
chunk); unknown keys under `config` are ignored. It is not a runtime env var or
setter — the value is fixed into the binary at build time.
[[src/manifest/mod.rs:stdin_log_cap]]

## Source Entries

Each element of `sources[]` is an object describing one source root:

| field | type | required | meaning |
| --- | --- | --- | --- |
| `root` | string | yes | directory (relative to the project) to scan; non-empty after trim |
| `include` | array of strings | no | glob patterns to include |
| `exclude` | array of strings | no | glob patterns to exclude |
| `role` | string | no | role tag (`"main"`/`"package"`); not validated here |

`root` is validated as a required, non-empty string. `include` and `exclude`,
when present, must each be an **array of strings** — any non-array value, or an
array containing a non-string element, is a `PROJECT_JSON_FIELD_TYPE` error.
A `sources[]` element that is not a JSON object is a `PROJECT_JSON_FIELD_TYPE`
error reported as `Source entry #N must be an object.`. [[src/manifest/mod.rs:validate_sources]] [[src/manifest/mod.rs:validate_source_pattern_field]]

The glob algorithm that turns these patterns into the `.mfb` input set is
`./mfb spec tooling source-selection`; `role` semantics are part of
that selection model.

## Dependency Entries

Each element of `packages[]` declares one dependency:

| field | type | default | meaning |
| --- | --- | --- | --- |
| `name` | string | — (required) | dependency package name; blank → entry ignored |
| `ident` | string | falls back to `name` | publisher identity of the dependency |
| `version` | string | `""` | requested version |
| `pin` | bool | `false` | when true, the installed `.mfp` must match `version` exactly |
| `source` | string | `""` | origin URL the dependency was added from |
| `identKey` | string | `""` | the pinned owner ident public key — the trust anchor. Written by `pkg add` on first add of a signed package (trust-on-first-use); every later build verifies the installed `.mfp` against this pin, never against the file-embedded key. Snake_case `ident_key` is accepted on read. |

`packages` must be an array when present (`validate_packages_array`); each
element must be an object with a string `name`. An entry whose `name` is absent,
non-string, or blank-after-trim is silently skipped. `pin = true` makes the
build reject an installed `packages/<name>.mfp` whose header version differs from
the declared `version`. [[src/manifest/package.rs:project_package_dependency]] [[src/manifest/package.rs:package_dependencies]] [[src/manifest/package.rs:installed_package_files]]

The registry/add workflow that writes these entries is
`./mfb spec package-manager`; the on-disk `.mfp` header they resolve
against is `./mfb spec package container-format`.

## Entry Point Validation

For `kind = "package"` no entry point is required and validation is skipped.
For `kind = "executable"` the manifest's `entry` (default `"main"`) must resolve
to exactly **one** top-level function across all source files, subject to these
rules — each failure emits `PROJECT_ENTRY_INVALID`: [[src/manifest/entry.rs:validate_entry_point]]

| condition | message gist |
| --- | --- |
| no function named `entry` | must declare an entry point named `<entry>` |
| more than one matching function | must declare exactly one entry point |
| `FUNC` entry whose return type ≠ `Integer` | entry `FUNC` must return Integer |
| a single parameter whose type ≠ `List OF String` | parameter must have type `List OF String` |
| more than one parameter | must declare zero params or one `args AS List OF String` |
| the `args` parameter declares a default value | args parameter must not declare a default |

A `SUB` entry is accepted (its `returns` is `Nothing`); a `FUNC` entry must
return `Integer`. The accepted parameter shapes are exactly: zero parameters, or
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
| `2-200-0010` | `PROJECT_JSON_VALID` | info | manifest passed validation |
| `2-200-0011` | `PROJECT_ENTRY_INVALID` | error | executable entry-point resolution failed [[src/manifest/entry.rs:validate_entry_point]] |

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
