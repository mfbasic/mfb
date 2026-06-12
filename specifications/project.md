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
      "version": "^2.1.0",
      "source": "registry:mfb"
    },
    {
      "name": "color",
      "version": "=0.4.2",
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
| `version` | string | Semantic version `MAJOR.MINOR.PATCH`. |
| `mfb` | string | Minimum compatible MFBASIC language version. |
| `sources` | array | Source entries included in the project. |

The `name` field must use the same identifier restrictions as MFBASIC package names unless a future registry specification defines a wider naming scheme. It must be the name used by `IMPORT name` in source code and by compiled `.mfp` package manifests.

The `version` field is required for both package and executable projects so lockfiles, build metadata, and generated packages can identify the exact project revision. Pre-release and build metadata follow semantic versioning when used, such as `1.2.0-beta.1` or `1.2.0+build.5`.

The `mfb` field names the minimum language version required to parse and type-check the source. A compiler may reject the project if it does not support that language version.

---

## 4. Identity And Informational Fields

| Field | Type | Required | Meaning |
| ----- | ---- | -------- | ------- |
| `name` | string | yes | Package import name. |
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

Tools may copy `name`, `version`, `author`, and `url` into `.mfp` package headers and manifests. A verifier must treat the compiled manifest, not `project.json`, as the source of truth for a compiled package.

---

## 5. Project Kind

The optional `kind` field declares the primary build intent:

| Value | Meaning |
| ----- | ------- |
| `package` | Build a reusable `.mfp` package. |
| `executable` | Build a native executable with an entry point named by `entry`, defaulting to `main`. |

If omitted, `kind` defaults to `executable` when a selected source package contains a valid entry point named by `entry` or by the default name `main`, and to `package` otherwise. Build tools should warn when inference is ambiguous.

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
  "version": "^3.0.0",
  "source": "registry:mfb",
  "platforms": ["macos-aarch64", "linux-x64"],
  "optional": false
}
```

| Field | Type | Required | Meaning |
| ----- | ---- | -------- | ------- |
| `name` | string | yes | Package identity imported by source code. |
| `version` | string | yes, except local path sources | Semantic-version constraint. |
| `source` | string | no | Package source locator. Defaults to the configured registry. |
| `alias` | string | no | Local resolver alias; does not change source `IMPORT` names. |
| `platforms` | array of strings | no | Target platforms where the package is active. |
| `optional` | boolean | no | Whether missing platform-specific package resolution may be skipped. |
| `hash` | string | no | Expected content hash for non-registry sources. |

Version constraints use the same forms as the language package rules: exact `=1.2.3`, compatible `^1.2.0`, patch-compatible `~1.2.0`, inequalities such as `>=1.2.0 <2.0.0`, or wildcard `1.2.*`.

Supported `source` forms:

| Form | Meaning |
| ---- | ------- |
| `registry:name` | Package from a named registry. |
| `path:relative/path` | Source package or `.mfp` file relative to `project.json`. |
| `file:relative/path/package.mfp` | Local compiled `.mfp` package. |
| `git+https://...` | Git repository fetched by the package manager. |
| `https://...` | Registry-specific package URL or package archive. |

Dependency `name` values must be unique within `packages`. Import graph cycles remain compile-time or bytecode merge-time errors.

The package resolver produces one selected version for each package identity. Executable builds must write or consume `mfb.lock`, which records exact selected package versions, source locators, content hashes, bytecode/package format versions, native dependency metadata hashes, and transitive dependencies.

---

## 8. Native Dependencies

Projects that expose `LINK` bindings should set `kind` to `binding` and may include `native` metadata. Application projects do not repeat a dependency package's native declarations; executable builds collect native requirements from selected packages.

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
| `targets` | array of strings | Requested build targets such as `native`, `bytecode`, or platform triples. |
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
- A package dependency has an invalid name, invalid version constraint, or unsupported source locator.
- Dependency resolution cannot select one compatible version per package identity.

Validation failure is a toolchain diagnostic. It is not recoverable by program `TRAP` code because no package code has started running.
