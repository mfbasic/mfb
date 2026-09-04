# bug-503: `project.json` `name` is path-joined unsanitized → `mfb build` writes an arbitrary-path 0755 executable

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (arbitrary file write / build-time supply chain)

Status: FIXED (see STATUS block below)

Regression Test: [x] `tests/syntax/project/project-name-traversal-error/` (golden pins `2-200-0017 PROJECT_JSON_NAME_INVALID`, `[exit 1]`); `cli::build::tests::build_project_rejects_a_traversing_project_name` (end to end: build fails, nothing appears beside the project, no `build/`); `manifest::tests::validate_project_manifest_rejects_a_name_that_is_not_a_path_component`; and one writer-level test per OS linker — `os::{linux::link,macos::link}::tests::refuses_to_write_an_executable_under_a_traversing_name`, `os::windows::tests::refuses_to_write_artifacts_under_a_traversing_name`.

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

## Fix

Two layers, both using the charset a `.mfp` package name already satisfies
(`validate_package_name`, `[A-Za-z0-9_][A-Za-z0-9_.-]*`; a census of the tree's
1,455 `project.json` files found only the LNK-12 spike non-conforming):

1. **Manifest gate** — `manifest::validate_name` runs inside
   `validate_project_manifest` (the sole gate every command passes) and emits the
   new hard error `2-200-0017 PROJECT_JSON_NAME_INVALID`. This covers *every*
   artifact writer at once — the live repro showed `-ast -ir` also wrote
   `<name>.ast`/`.ir` outside the project, not just the 0755 executable.
2. **Defence in depth at the filesystem boundary** — `os::validate_output_name`
   is called at the top of every writer that `Path::join`s the name
   (`linux::link::write_executable`, `linux::appdir::write_appdir`,
   `linux::appimage::seal`, `macos::link::{write_executable,write_app_bundle}`,
   `windows::write_linked_executable`, all three `write_native_object_plan`s) and
   in every shared `target::write_*` dispatcher before a backend runs, so an
   `IrProject` constructed without the manifest cannot bypass the gate.

Spec synced: `tooling/01_project-manifest.md` (`name` row + rule table) and
`diagnostics/01_rule-codes.md` (`rules::tests::every_rule_is_documented_in_the_spec`
gates the latter).

STATUS: FIXED (e096f25d8)
Reproduced first with the unfixed binary (`mfb build /tmp/lnk12` → 0755 Mach-O at
`/tmp/lnk12-pwn/evil.out`); after the fix the same command exits 1 with
`2-200-0017` and `/tmp/lnk12-pwn/` stays empty, no `build/` is created. No
deviation from the Best fix; the `GUARD_CF`/LNK-14 and bug-504 items are separate.
