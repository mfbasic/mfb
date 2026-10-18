# bug-1102: `mfb man os name` and `mfb spec stdlib os` say `os::name` is `"macos"` or `"linux"`; a Windows build returns `"windows"`

Last updated: 2026-10-17
Effort: small (<1h)
Severity: LOW
Class: Documentation (spec and man drift)

Status: Open
Regression Test: none (prose); verify by rendering `mfb man os name` and `mfb spec stdlib os`

Found by the bug-staging-24 coordinator while reading `os::name` for bug-1017, and placed in a
later phase.

**The single correct behavior a fix produces:** both documents list all three values the
target family can produce: `"macos"`, `"linux"` and `"windows"`.

## Reproduction (main `f31ec00b7`)

```
mfb man os name      # Description: "... of the build target: "macos" or "linux"."
mfb spec stdlib os   # compile-time constants table: | `os::name` | `"macos"` or `"linux"` ...
```

A program that prints `os::name()`, built with `mfb build -q -target windows-x86_64` and run on
box 206 (Windows 11), prints `windows`. The value comes from
`src/codegen/builtins/os/gen_shared.rs:os_family` (`PlatformFamily::Windows => "windows"`), and
the doc comment on `func_name.rs:lower_name` already says `"macos"`/`"linux"`/`"windows"`.

## Where

- `src/codegen/builtins/os/func_name.rs` `DESC` (the man page).
- `src/docs/spec/stdlib/14_os.md`, the compile-time constants table row for `os::name`.
