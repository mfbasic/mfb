# Object Plan

The object plan is a second, independent model of the native image, separate
from the bytes the linker actually emits. It is a JSON-serializable description
of the *planned* container — segments, sections, symbol table, relocations — and
exists to validate that the `NativePlan` is structurally sound before the code
plan and linker materialize real bytes. It is built by
`os::<platform>::object::lower_plan` from a `NativePlan`.

The object plan is never consumed by the linker. The linker works only from the
`EncodedImage`. The two models are deliberately parallel: the object plan
catches structural errors (undefined symbols, overlapping sections, missing
entry) cheaply and independently of the byte encoder.

## When it runs

`lower_plan` is invoked in two situations:

- As a validation gate during every executable build:
  `os::<platform>::validate_native_object_plan(&native_plan)` builds the object
  plan and runs `validate()` on it, discarding the JSON. This runs after
  `native_plan.validate()` and before code lowering.
- As an inspectable artifact: `mfb build` with the native-object-plan output
  selection writes `<project>.nobj` (the `lower_plan` result serialized via
  `to_json`). Normal executable builds do not write this file.

Because the object plan is a parallel model and not an input to the linker, any
change to symbol naming, section layout, or relocation kinds must be reflected in
*both* `object.rs` and `link.rs` to keep the gate meaningful.

## Shape

`NativeObjectPlan` (`src/os/macos/object.rs`, `src/os/linux/object.rs`) records:

```text
target            "macos-aarch64" | "linux-aarch64"
container         "mach-o" | "elf"
status            "planOnly"
entry             "_main"
image_base        VM base address
dylibs            referenced libraries (macOS; empty on Linux)
load_commands     planned Mach-O load commands (macOS)
segments          planned segments with vm/file ranges
sections          planned sections with offset/size/align
code_units        one per function: symbol, size, operations, calls, data refs
data_units        one per data object: symbol, size, value
defined_symbols   internal text/data symbols
imported_symbols  (library, symbol) external imports
external_symbols  external relocation targets
symbol_table      name/kind/section/value/string-table-offset entries
string_table      symbol string table
relocations       from/to/kind/section records
```

The JSON form is tagged `"mfb-native-object-plan"`, version 2. Each call's
`CallKind` maps to an object-plan relocation kind: `Local` and `Runtime` become
`internalCall`, `Import` becomes `packageCall` (a legacy label for native `LINK`
thunk calls), and `Indirect` becomes `indirectCall`. Data references and the
`_main` error-message references are recorded as `dataReference` relocations.

## Validation checks

`NativeObjectPlan::validate()` enforces, per platform:

- `target` matches the platform (`"macos-aarch64"` / `"linux-aarch64"`).
- `container` is the expected format and `status` is `"planOnly"`.
- The entry symbol `_main` is present in `defined_symbols`.
- `defined_symbols` has no duplicates (and, on macOS, `dylibs` has no
  duplicates).
- Every relocation's source is a defined symbol, and its target is defined,
  imported, or external.

macOS additionally checks section layout:

- The `__TEXT,__text` section is present.
- No two non-empty sections overlap in their file ranges (the `__LINKEDIT`
  section is allowed to carry zero size).
- Every code unit has a non-empty operation list.

A failed check aborts the build before any bytes are encoded.
