# bug-503: `project.json` `name` is path-joined unsanitized → `mfb build` writes an arbitrary-path 0755 executable

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (arbitrary file write / build-time supply chain)

Status: Open (found in audit-3, Surface 7 LNK-12; reproduced live by the lead)

Regression Test: a build fixture with `"name": "../evil"` asserting a name-validation error, not an out-of-tree write.

## Summary

Building an untrusted MFBASIC project should not write anything outside the
project directory (`mfb build` does not even run the program). But the project
`name` from `project.json` is interpolated into the output path with a plain
`Path::join` and no component validation — `../` escapes the tree and a leading
`/` is absolute — and the result is `chmod 0755`. Cloning an untrusted repo /
template and running `mfb build` therefore plants an executable at an
attacker-chosen path (e.g. `~/.local/bin/`, an autostart dir).

## Mechanism

```rust
// src/os/linux/link/mod.rs:74
let path = out_dir.join(format!("{project_name}-{}.out", flavor.suffix()));
// src/os/linux/appdir.rs:70   (app-mode inner binary, no suffix at all)
let executable = bin_dir.join(project_name);
// src/os/macos/link/mod.rs:86 (same shape)
let executable_path = macos_dir.join(project_name);
```

then `chmod 0755` (`linux/link/mod.rs:83`, `macos/link/mod.rs:199`,
`linux/appdir.rs:194`). The validator that closes this class exists but is applied
only on the *package* path — `validate_package_name` (`src/manifest/package.rs:58`)
— never on the executable path (`grep -rn validate_package_name src/` shows no
`src/os/**` caller).

## Reproduction (lead-run, live)

`spikes/audit-3/LNK-12/` (project `name = "../../../../../../tmp/lnk12-pwn/evil"`):

```
mkdir -p /tmp/lnk12-pwn
mfb build spikes/audit-3/LNK-12
# Wrote executable to /tmp/lnk12/build/../../../../../../tmp/lnk12-pwn/evil.out
ls -l /tmp/lnk12-pwn/    # evil.out, 0755, Mach-O 64-bit executable arm64
```

The file lands entirely outside the project. (A leading `/` gives a fully
absolute target.)

## Best fix

Validate `ir.name` with `validate_package_name` (or a dedicated
`validate_output_name`) at the top of the executable/app-bundle writers on all
targets, rejecting any name containing a path separator, `..`, or a leading `.`
before it reaches `Path::join`. This is the same check the package path already
enforces.

## Non-goals

Do not change the output filename shape for well-formed names; no manifest wire
change (the field stays a string, just validated).

## Prior art

None for the executable path (searched `project name`, `path join`, `traversal`,
`validate_package_name`, `write_executable`). bug-395 fixed a related traversal on
the re-exported foreign-owner name in `binary_repr`; the os/link executable path
was uncovered.
