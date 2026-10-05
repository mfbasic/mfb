# bug-959: `LINK … AS win` with a function named `pathbuf`/`vendorsep` fails the Windows vendoring build with an internal duplicate-symbol error

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: measured by the review agent on the Windows box only; the coordinator did not re-run it.
Regression Test: src/codegen/builtins/tests/link.rs (new: Windows vendoring build with alias `win` / fn `pathbuf`)

The Windows `_mfb_linker_init` support emits two fixed internal data symbols when the build
vendors a native library: `_mfb_linker_win_pathbuf` (`WIN_LINK_PATHBUF_SYMBOL`) and
`_mfb_linker_win_vendorsep` (`WIN_LINK_VENDORSEP_SYMBOL`). They live in the same namespace as
the per-function marshaling thunks `_mfb_linker_<alias>_<name>`, and the thunk name for
`LINK "…" AS win` / `FUNC pathbuf` is exactly `_mfb_linker_win_pathbuf`. A valid program is
therefore rejected with an unlocated internal error on `windows-x86_64` only (the same source
builds and runs on macOS). The spec promises the escaping scheme keeps thunk names
collision-free; the reserved names were never escaped out of the thunk space.

Correct behavior: any legal `alias`/`name` pair builds on every target; internal linker data
symbols use a spelling no `link_thunk_symbol` output can produce.

References:

- `mfb spec linker package-linking` ("each alias/name component is escaped so every byte that
  is not [A-Za-z0-9] … becomes a `_XX` escape, so an interior `_` cannot collide")
- `mfb spec linker windows-x86_64` ("Native LINK bindings and vendored DLLs":
  `_mfb_linker_win_pathbuf`)

## Failing Reproduction

```
mkdir -p wcoll/src wcoll/vendor && echo dummy > wcoll/vendor/foo.dll
cat > wcoll/project.json <<'EOF'
{ "name": "wcoll", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "libraries": { "foo": [ { "os": "windows", "type": "vendor", "source": "foo.dll" },
                          { "os": "macos", "type": "system", "source": "libfoo.dylib" } ] },
  "sources": [ { "root": "src", "role": "main", "include": ["**/*.mfb"] } ],
  "entry": "main", "targets": ["native"] }
EOF
cat > wcoll/src/main.mfb <<'EOF'
IMPORT io
LINK "foo" AS win
  FUNC pathbuf(a AS Integer, b AS Integer) AS Integer
    SYMBOL "foo_add"
    ABI (a CInt32, b CInt32) AS r CInt32
    RETURN r
  END FUNC
END LINK
FUNC main() AS Integer
  io::print("add=" & toString(win::pathbuf(2, 3)))
  RETURN 0
END FUNC
EOF
/tmp/hardening/mfb-head build --target windows-x86_64 wcoll
```

- Observed: `error: native code plan defines symbol '_mfb_linker_win_pathbuf' more than once`
  (no source location). Renaming the function to `vendorsep` gives the same error for
  `_mfb_linker_win_vendorsep`.
- Expected: `Wrote executable to wcoll/build/wcoll.exe`.

Contrast (measured): the same source with a Windows `system` locator (no vendoring → the
buffer is not emitted) builds; the macOS build of the vendoring project builds and prints
`add=1005` against a real vendored dylib.

## Root Cause

`src/codegen/link/thunk/link_thunk.rs:WIN_LINK_PATHBUF_SYMBOL` / `WIN_LINK_VENDORSEP_SYMBOL`
are raw `_mfb_linker_win_<word>` strings, while
`src/target/shared/nir/mod.rs:link_thunk_symbol` produces `_mfb_linker_<esc(alias)>_<esc(name)>`
— for alias `win` and an alphanumeric name the escape is the identity, so the two coincide.
The code-plan validator then rejects the duplicate.

## Goal

- The repro builds; the thunk and the scratch buffer get distinct symbols.

### Non-goals (must NOT change)

- The thunk naming/escaping scheme for every other alias (goldens keyed on it).

## Blast Radius

- `WIN_LINK_PATHBUF_SYMBOL`, `WIN_LINK_VENDORSEP_SYMBOL` — fixed by this bug (e.g. a sigil or
  a different prefix such as `_mfb_linkerrt_…`).
- `_mfb_linker_lib_<n>`, `_mfb_linker_sym_<n>`, `_mfb_linker_free_<n>` — unaffected: the
  trailing part is a decimal index and no MFBASIC identifier can start with a digit (guess —
  not exercised).
- `_mfb_linker_init` — unaffected: a thunk name always carries `_<name>` after the alias.
