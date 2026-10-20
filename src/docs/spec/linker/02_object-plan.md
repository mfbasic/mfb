# Object Plan

The object plan is a second, independent model of the native image, separate
from the bytes the linker actually emits. It is a JSON-serializable description
of the *planned* container — segments, sections, symbol table, relocations — and
exists to validate that the `NativePlan` is structurally sound before the code
plan and linker materialize real bytes. It is built from a `NativePlan` by the
platform object-plan builder.
[[src/os/linux/object.rs:lower_plan]]

The object plan is never consumed by the linker. The linker works only from the
`EncodedImage`. The two models are deliberately parallel: the object plan
catches structural errors (undefined symbols, overlapping sections, missing
entry) cheaply and independently of the byte encoder.

## When it runs

The object-plan builder is invoked in two situations:

- As a validation gate during every executable build: the platform object-plan
  validator builds the object plan and validates it, discarding the JSON. This
  runs after the native-plan validation and before code lowering.
- As an inspectable artifact: `mfb build` with the native-object-plan output
  selection writes `<project>.nobj` (the builder result serialized to JSON).
  Normal executable builds do not write this file.

The object model describes structural relationships, not the final byte layout.
For example, the Linux model uses a fixed `image_base` and one RX segment while
the dynamic ELF writer emits a PIE with separate protection regions. Windows
uses a synthetic `_start` entry in this model; the encoded image uses `_main`.
Final offsets, protections, and container metadata are determined by the linker.
[[src/os/linux/object.rs:lower_plan]] [[src/os/windows/object.rs:lower_plan]]
[[src/codegen/engine/builder/lower_module.rs:lower_module_for_platform]]

## Shape

`NativeObjectPlan` records:
[[src/os/macos/object.rs]] [[src/os/linux/object.rs]]

```text
target            "macos-aarch64" | "linux-aarch64" | "linux-x86_64" | "linux-riscv64" | "windows-x86_64"
container         "mach-o" | "elf" | "pe"
status            "planOnly"
entry             "_main" (macOS/Linux), "_start" (Windows)
image_base        VM base address
dylibs            referenced libraries (macOS; empty on Linux)
dlls              referenced DLLs (Windows, instead of dylibs)
load_commands     planned Mach-O load commands (macOS)
segments          planned segments with vm/file ranges (macOS/Linux)
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
[[src/os/linux/object.rs:NativeObjectPlan]] [[src/os/windows/object.rs:NativeObjectPlan]]

The JSON form is tagged `"mfb-native-object-plan"`, version 2. A call's
`CallKind` maps to an object-plan relocation kind: `Local` and `Runtime` become
`internalCall`, and `Import` becomes `packageCall` (native `LINK` thunk calls). An `Indirect` call produces *no relocation*: it dispatches
through a `FUNC`-typed runtime value (a local, parameter, or lambda binding) and
has no linker symbol, so its plan-call record carries an empty `symbol` and the
machine-code path emits a genuine indirect branch through the callable value.
[[src/os/macos/object.rs:relocations]] Data references and the `_main`
error-message references are recorded as `dataReference` relocations.

## Validation checks

`NativeObjectPlan::validate()` enforces, per platform:

- `target` matches the platform. macOS accepts `"macos-aarch64"`; the Linux
  validator accepts **any** Linux target — `"linux-aarch64"`, `"linux-x86_64"`, or
  `"linux-riscv64"` — because the object/ELF plan is ISA-neutral: an ELF container
  is an ELF container, and nothing at this layer varies by instruction set.
  [[src/os/linux/object.rs:validate]]
- `container` is the expected format and `status` is `"planOnly"`.
- Windows accepts only `"windows-x86_64"` with container `"pe"`.
- The model entry (`_main` on macOS/Linux, `_start` on Windows) is present in
  `defined_symbols`. [[src/os/windows/object.rs:validate]]
- `defined_symbols` has no duplicates (and, on macOS, `dylibs` has no
  duplicates).
- Every relocation's source is a defined symbol, and its target is defined,
  imported, or external.

[[src/os/linux/object.rs:validate]]

macOS additionally checks section layout:

- The `__TEXT,__text` section is present.
- No two non-empty sections overlap in their file ranges (the `__LINKEDIT`
  section is allowed to carry zero size).
- Every code unit has a non-empty operation list.

A failed check aborts the build before any bytes are encoded.

## See Also

* ./mfb spec linker pipeline — where the object-plan gate sits in the stage sequence
* ./mfb spec linker failure-rules — the structural errors this gate rejects
* ./mfb spec linker symbols-and-relocations — the symbol and relocation model it validates
