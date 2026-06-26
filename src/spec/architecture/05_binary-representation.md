# Binary Representation and Package Generation

The Binary Representation: the IR exposed as a versioned on-disk contract, and the MFP package container.

Binary Representation generation is implemented in `src/binary_repr.rs`.
MFP package wrapping is implemented in `src/target/package_mfp/mod.rs`.

## What the Binary Representation Is

**The Binary Representation is the compiler's IR, exposed as a versioned external
interface.** The in-memory IR (see the `ir` topic — `IrProject` / `IrFunction` /
`IrOp` / `IrValue` / `IrType`) is the compiler's private, in-process model and is
free to change between builds. The Binary Representation is a defined, versioned
binary *serialization* of that model: control flow stays nested, expressions stay
as trees, and the structure is preserved faithfully — there is no lowering to a
flat opcode/register machine. `src/binary_repr.rs` encodes IR → Binary
Representation and decodes Binary Representation → IR.

The two are related but **not the same thing**, and the distinction is the whole
point of the boundary:

- The **IR** is an unstable in-memory data structure. Nothing outside the
  compiler process may depend on its layout.
- The **Binary Representation** is the stable on-disk contract. It carries its own
  format version (`MFBR` payload magic, `MFPC` container major `2`), so a future
  compiler can change the IR freely as long as it can still encode/decode this
  versioned format. Because the encoding is a faithful, structure-preserving
  serialization, a consumer **decodes it straight back into IR** and lowers it
  through the single `IR → NIR → native` codegen used for the executable's own
  code — no second, package-only code path.

The binary representation layer lowers IR into an architecture-independent package
image that starts with `MFPC` magic and contains sectioned data. The implemented
sections include:

- manifest
- string pool
- type table
- constant pool
- import table
- export table
- global table
- function table
- binary representation (the structured function bodies, `MFBR` payload)
- resource table
- ABI index
- documentation table (section id `17`, written only when the package carries
  `DOC` documentation)

The binary representation writer builds:

- A string pool for names, literals, package metadata, and version data.
- A type table with primitive and user-defined types.
- A constant pool for literal values.
- Import and dependency metadata.
- Export metadata for non-private functions.
- Function tables with parameters and cleanup metadata; each function body is the
  structured Binary Representation node tree in the `MFBR` payload section.
- ABI hashes used by package readers and dependency checks.

`mfb build -br` writes a hexadecimal dump of the binary representation to `<project>.hex`.
When the executable project has package dependencies, the binary representation path
decodes installed packages back into IR and merges them, so every function flows
through the one codegen.

## MFP Package Container

Package projects emit a `.mfp` file through `target::write_package`.

The package path is:

```text
IR
  -> binary_repr::build_binary_repr_bytes
  -> package_mfp::build_package_bytes
  -> <package>.mfp
```

Package metadata is derived from `project.json`:

- `name`
- `version`
- `author`
- `url`
- dependency constraints from `packages`

The package writer emits an MFP container with:

- container major/minor: `1.0`
- binary representation major/minor: `1.0`
- pre-release flag set when the version contains `-`

Signing is selectable. Without `--sign`, `write_package` calls
`build_package_bytes`, which emits an unsigned container (`signatureType = 0`,
`signatureLength = 0`). With `--sign owner`, it calls `build_signed_package_bytes`,
which signs the payload and emits an ed25519 header (`signatureType = 1`,
`signatureLength = 64`). The reader's `validate_signature_header` accepts both
forms.

The package payload must start with `MFPC`. Metadata string lengths are checked
before writing.

## Error Source Locations

Every user-visible `Error` carries an `ErrorLoc source` recording where it
originated. The location flows through every layer:

- **AST** (`src/ast.rs`): `Expression::Call`/`Binary`/`Unary` and `Statement::For`
  carry an internal `(line, column)`; the source file is the enclosing `AstFile`.
  These are not serialized to the `.ast` JSON.
- **IR** (`src/ir.rs`): `IrValue::Call`/`CallResult`/`Binary`/`Unary` and
  `IrOp::For` carry an `IrSourceLoc { line, column }`; each `IrFunction` carries
  its source `file`. The `error(code, message)` built-in lowers to nested record
  constructors — `Error[code, message, ErrorLoc[file, line, char]]` — so
  `Error`/`ErrorLoc` are ordinary records for the rest of the pipeline. These
  fields are not serialized to the `.ir` JSON but are encoded into the Binary
  Representation, so an imported package's functions retain their own source
  locations.
- **NIR** (`src/target/shared/nir.rs`): mirrors the IR fields (`NirSourceLoc`,
  `NirFunction::file`).
- **Native runtime** (`src/target/shared/code`): the code generator tracks the
  current function file and the current node location, builds a real `ErrorLoc`
  at every error origin (user `error(...)`, arithmetic overflow/divide-by-zero,
  failing built-in/helper calls), and threads the origin through the four-register
  result ABI (see `memory_layouts.md`). Propagation preserves the origin; trapping
  materializes the 3-field `Error`.
