# `project.json` Project Manifest

`project.json` is the source-level manifest for an MFBASIC project. It gives the project a stable identity, names its source inputs, declares package dependencies, and records build metadata used by compilers, package managers, language servers, and audit tools.

The project manifest is an authoring file. It is not embedded verbatim in `.mfp` output. Compilers copy the relevant identity, dependency, native-link, language-version, and audit metadata into the compiled package manifest described by the `.mfp` package format.

`project.json` files are UTF-8 JSON. They must not contain comments or trailing commas.

---

## 1. Design Goals

1. **Stable project identity** - the importable package name is independent of local directory names.
2. **Explicit inputs** - builds read only the source roots and files selected by the manifest.
3. **Package-manager friendly** - dependencies are declared in a predictable, lockfile-compatible shape.
4. **Tooling friendly** - editors, formatters, test runners, and audit tools can discover project intent without compiling source.
5. **Portable by default** - host-specific native dependencies, permissions, and targets are explicit.

---

## 2. Example

```json
{
  "name": "geometry",
  "ident": "ada#geometry",
  "version": "0.1.0",
  "description": "2D and 3D geometry helpers for MFBASIC.",
  "author": "Ada Lovelace <ada@example.com>",
  "repository": {
    "type": "git",
    "url": "https://example.com/ada/geometry.git"
  },
  "url": "https://example.com/geometry",
  "license": "MIT",

  "mfb": "1.0",
  "kind": "package",

  "sources": [
    {
      "root": "src",
      "include": ["**/*.mfb"],
      "exclude": ["**/*_test.mfb"]
    },
    {
      "root": "tests",
      "role": "test",
      "include": ["**/*.mfb"]
    }
  ],

  "packages": [
    {
      "name": "shape",
      "ident": "ada#shape",
      "version": "2.1.0",
      "source": "registry:mfb"
    },
    {
      "name": "color",
      "ident": "ada#color",
      "version": "0.4.2",
      "pin": true,
      "source": "git+https://example.com/mfb/color.git"
    }
  ],

  "entry": "main",
  "targets": ["native"]
}
```

---

## 3. Required Fields

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `name` | string | Package import name used by source code. |
| `ident` | string | Registry identity `<owner>#<package>` for published packages. |
| `version` | string | Semantic version `MAJOR.MINOR.PATCH`. |
| `mfb` | string | Minimum compatible MFBASIC language version. |
| `kind` | string | Primary build intent: `executable` or `package`. |
| `sources` | array | Source entries included in the project. |

The `name` field must use the same identifier restrictions as MFBASIC package names. It is the name used by `IMPORT name` in source code and by compiled `.mfp` package manifests. It is not globally unique and is not sufficient to resolve a registry dependency.

The `ident` field is the registry identity for a package that will be published or resolved through a registry. It uses the repository identity form `<owner>#<package>`. The `<package>` slug may differ from `name`. Executable-only projects and private local packages may omit `ident`; tools that publish a package to a registry must require it and copy it into the compiled `.mfp` manifest.

The `version` field is required for both package and executable projects so lockfiles, build metadata, and generated packages can identify the exact project revision. Pre-release and build metadata follow semantic versioning when used, such as `1.2.0-beta.1` or `1.2.0+build.5`.

The `mfb` field names the minimum language version required to parse and type-check the source. A compiler may reject the project if it does not support that language version.

---

## 4. Identity And Informational Fields

| Field | Type | Required | Meaning |
| ----- | ---- | -------- | ------- |
| `name` | string | yes | Package import name. |
| `ident` | string | package publish | Registry identity `<owner>#<package>`. |
| `version` | string | yes | Project/package version. |
| `description` | string | no | Human-readable summary. |
| `author` | string or object | no | Human-readable author metadata. |
| `repository` | string or object | no | Source repository. |
| `url` | string | no | Project homepage or documentation URL. |
| `license` | string | no | SPDX license expression or `UNLICENSED`. |

`author` may be a string or an object:

```json
{
  "name": "Ada Lovelace",
  "email": "ada@example.com",
  "url": "https://example.com/ada"
}
```

`repository` may be a URL string or an object:

```json
{
  "type": "git",
  "url": "https://example.com/ada/geometry.git",
  "directory": "packages/geometry"
}
```

Tools may copy `name`, `ident`, `version`, `author`, and `url` into `.mfp` package headers and manifests. A verifier must treat the compiled manifest, not `project.json`, as the source of truth for a compiled package.

---

## 5. Project Kind

The required `kind` field declares the primary build intent:

| Value | Meaning |
| ----- | ------- |
| `package` | Build a reusable `.mfp` package. |
| `executable` | Build a native executable with an entry point named by `entry`, defaulting to `main`. |

Build tools must reject a manifest that omits `kind`. They must not infer `executable` or `package` from the presence or absence of an entry point.

For `executable` projects, `entry` defaults to `"main"`. The entry point names a root-package declaration with one of the accepted executable signatures:

```basic
SUB entry
END SUB

SUB entry(args AS List OF String)
END SUB

FUNC entry AS Integer
END FUNC

FUNC entry(args AS List OF String) AS Integer
END FUNC
```

Empty parentheses are also valid for zero-argument entries. The single optional argument receives the command-line argument vector, where `get(args, 0)` is the program name as invoked by the host. Multiple matching entry points, no matching entry point, any other parameter list, or a `FUNC` entry whose success type is not `Integer` are compile-time errors.

---

## 6. Sources

The `sources` field is an ordered array of source entries. Each entry selects `.mfb` files from one root.

```json
{
  "root": "src",
  "role": "main",
  "include": ["**/*.mfb"],
  "exclude": ["**/*_test.mfb"],
  "package": "geometry"
}
```

| Field | Type | Required | Meaning |
| ----- | ---- | -------- | ------- |
| `root` | string | yes | File or directory path relative to `project.json`. |
| `role` | string | no | `main`, `test`, `example`, `tool`, or implementation-defined. |
| `include` | array of strings | no | Glob patterns relative to `root`. |
| `exclude` | array of strings | no | Glob patterns relative to `root`. |
| `package` | string | no | Explicit package name for files directly under `root`. |

If `root` is a file, it must name a `.mfb` source file and `include` is ignored. If `root` is a directory and `include` is omitted, the default is `["**/*.mfb"]`.

Source paths must stay inside the project directory after path normalization unless a build tool is explicitly invoked with a policy that allows external paths. Symlinks are resolved before this check.

Directory-to-package rules from the language specification still apply: each source directory contributes to one package namespace. The optional `package` field gives the package identity for files directly under the source root when the directory name is not the desired import name. Child directories still use their own directory names as package names unless a future workspace specification adds explicit package mapping.

Files selected by more than one `main` source entry are an error. Test, example, and tool source entries may overlap main sources only when the build tool defines that behavior explicitly.

---

## 7. Packages

The `packages` field declares package dependencies. It is an array, not an object, so each dependency can carry source, trust, and platform metadata without inventing nested field names.

```json
{
  "name": "sqlite",
  "ident": "data#sqlite",
  "version": "3.0.0",
  "pin": false,
  "source": "registry:mfb",
  "platforms": ["macos-aarch64", "linux-x64"],
  "optional": false
}
```

| Field | Type | Required | Meaning |
| ----- | ---- | -------- | ------- |
| `name` | string | yes | Import name used by source code. |
| `ident` | string | yes for registry sources | Registry identity `<owner>#<package>`. |
| `version` | string | yes, except local path sources | Requested concrete semantic version. |
| `pin` | boolean | no | If true, resolve exactly `version`; otherwise resolve the highest ABI-compatible version anchored at `version`. |
| `source` | string | no | Package source locator. Defaults to the configured registry. |
| `alias` | string | no | Local resolver alias; does not change source `IMPORT` names. |
| `platforms` | array of strings | no | Target platforms where the package is active. |
| `optional` | boolean | no | Whether missing platform-specific package resolution may be skipped. |
| `hash` | string | no | Expected content hash for non-registry sources. |

The `version` field is a requested version, not a range expression. It must be a concrete semantic version such as `3.0.0`, without `^`, `~`, `=`, inequality operators, wildcards, or other range syntax.

When `pin` is omitted or false, the resolver treats `version` as an ABI anchor: it may select the highest available package version whose `ABI_INDEX` is compatible with the requested version. When `pin` is true, the resolver must select exactly `version` and the matching content hash.

The `ident` field is the resolver identity. The `name` field is the source import name. Two dependencies may have the same `name` only if they are never active in the same build and the build tool has a defined aliasing policy; otherwise dependency `name` values must be unique within active `packages`.

Supported `source` forms:

| Form | Meaning |
| ---- | ------- |
| `registry:name` | Package from a named registry. |
| `path:relative/path` | Source package or `.mfp` file relative to `project.json`. |
| `file:relative/path/package.mfp` | Local compiled `.mfp` package. |
| `git+https://...` | Git repository fetched by the package manager. |
| `https://...` | Registry-specific package URL or package archive. |

Import graph cycles remain compile-time or binary representation merge-time errors.

The package resolver produces one selected version for each package `ident`. Executable builds must write or consume `mfb.lock`; its format is specified in `lockfile.md`.

### Package Trust Policy

Unsigned `.mfp` packages are rejected by default. The public registry source `registry:mfb` and all remote sources must reject unsigned packages unconditionally.

Local development may opt into unsigned local packages with an explicit policy:

```json
{
  "packagePolicy": {
    "allowUnsignedLocal": true
  }
}
```

`allowUnsignedLocal` applies only to `path:` and `file:` sources. It does not apply to `registry:*`, `git+https://...`, `https://...`, mirrors, or cached registry blobs. When this policy permits an unsigned local package, `mfb.lock` must record the unsigned-local exception for that package.

---

## 8. Native Dependencies

Projects that expose `LINK` bindings use `kind: "package"` and may include `native` metadata. Application projects do not repeat a dependency package's native declarations; executable builds collect native requirements from selected packages.

```json
{
  "native": [
    {
      "name": "sqlite3",
      "version": ">=3.45.0",
      "platforms": ["macos-aarch64", "linux-x64"],
      "link": "dynamic",
      "headers": ["sqlite3.h"]
    }
  ]
}
```

`native` metadata is informational to source tools and package managers until a matching `LINK` declaration is compiled. The compiled `.mfp` native-link metadata is authoritative for importers.

Built-in standard packages may require platform baseline libraries without user manifest entries. For example, macOS executable builds can satisfy built-in file I/O through the OS baseline `libSystem`; applications do not declare that dependency manually unless they expose their own `LINK` binding.

---

## 9. Targets And Build Metadata

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `targets` | array of strings | Requested build targets such as `native`, `binary representation`, or platform triples. |
| `kind` | string | Required project kind: `executable` or `package`. |
| `entry` | string | Executable entry point symbol, defaulting to `main`. |
| `build` | object | Toolchain-specific build settings. |

The `build` object is reserved for toolchain configuration that affects generated artifacts, such as optimization level, debug metadata, source maps, audit metadata, output directory, and target platform. Unknown build keys must be ignored by tools that do not own them unless they are under a required namespace defined by that tool.

---

## 10. Unknown Fields

Tools must ignore unknown top-level fields by default. Vendor-specific fields should use one of these forms:

```json
{
  "x-vendor": {},
  "tool": {
    "vendor": {}
  }
}
```

A future manifest version may reserve additional top-level fields. Tool-specific required behavior should be namespaced so ordinary compilers can still parse and build the project when they do not need that behavior.

---

## 11. Validation

A tool must reject `project.json` when:

- The file is not valid UTF-8 JSON.
- A required field is missing.
- `name` is not a valid package import name.
- `version` is not a semantic version.
- `mfb` requires an unsupported language version.
- A source path escapes the project root under the active path policy.
- No source files are selected for the requested build.
- Two main source entries select the same source file.
- A package dependency has an invalid name, invalid ident, invalid requested version, invalid `pin` value, or unsupported source locator.
- `packagePolicy.allowUnsignedLocal` is used for any source other than `path:` or `file:`.
- Dependency resolution cannot select one compatible version per package ident.

Validation failure is a toolchain diagnostic. It is not recoverable by program `TRAP` code because no package code has started running.
