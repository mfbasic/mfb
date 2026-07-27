# Binary Representation and Package Generation

The Binary Representation: the IR exposed as a versioned on-disk contract, and the MFP package container.

Binary Representation generation is handled by the binary-representation layer.[[src/binary_repr/]]
MFP package wrapping is handled by the package writer.[[src/target/package_mfp/mod.rs]]

## What the Binary Representation Is

**The Binary Representation is the compiler's IR, exposed as a versioned external
interface.** The in-memory IR (see the `ir` topic — `IrProject` / `IrFunction` /
`IrOp` / `IrValue` / `IrType`) is the compiler's private, in-process model and is
free to change between builds. The Binary Representation is a defined, versioned
binary *serialization* of that model: control flow stays nested, expressions stay
as trees, and the structure is preserved faithfully — there is no lowering to a
flat opcode/register machine. The binary-representation layer encodes IR → Binary
Representation and decodes Binary Representation → IR.[[src/binary_repr/]]

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
  code — no second, package-only code path.[[src/binary_repr/reader.rs:read_binary_repr_package]]

The binary representation layer lowers IR into an architecture-independent package
image that starts with `MFPC` magic and contains sectioned data — a string pool,
type table, constant pool, import/export tables, global and function tables, the
structured function bodies (`MFBR` payload), a resource table, an ABI index, and
an optional documentation table. The exact section catalog, section ids, and
byte encodings are owned by `./mfb spec package container-format` and
`./mfb spec package doc-section`.

Architecturally, the writer's job is to project the in-memory IR into that
sectioned form: names/literals/metadata into the string pool, primitive and
user-defined types into the type table, literal values into the constant pool,
import/export and dependency metadata, function tables with parameters and
cleanup metadata, and the ABI hashes package readers use for dependency checks.

`mfb build --br` writes a hexadecimal dump of the binary representation to `<project>.hex`.

## Decode-and-Merge of Package Dependencies

This is the canonical description of how a native executable build folds its
installed `.mfp` dependencies back into IR. Because the Binary Representation is
a faithful, structure-preserving serialization of IR, an executable build does
**not** keep package bodies as external symbols: package merging
decodes each installed package's binary
representation back into IR, prefixes every package symbol with a per-package
identity, merges the functions, types, globals, and
constants into the application IR, and rewrites the consumer's `package.symbol`
references to the identity-prefixed definitions.
Package functions therefore flow through the single `IR → NIR → native` codegen
as ordinary merged functions (emitted under the normal `_mfb_fn_…` symbol
namespace), not as `_mfb_pkg_*` imports. The only true NIR imports are native
`LINK` thunks and platform symbols.[[src/target/shared/nir/lower.rs:merge_packages]]

The per-package identity that `read_package_ir_with_identity` produces is a hash
over the MFPC container; its byte derivation is documented in
`./mfb spec package ir-section`.

## Re-exporting a Dependency's Type

A package's own exported API may name a type it imported from a declared
dependency — for example `EXPORT FUNC takesA(a AS A)` where `A` is exported by a
dependency `pA`. The package does **not** copy `A`'s definition into its own
`.mfp`; a copied definition would clash with `pA`'s own definition when an
executable later merges both, and would sever the link to `pA`'s version.

Instead the type table records a **foreign type reference**: an entry naming the
owning dependency, the type's original name, and the owning package's ABI hash
for that type (as `pA` itself computed it). The reference is serialized into the
referring package's ABI hashes by that owning identity — never by re-walking the
(absent) fields — so the same `pA::A` surfaced through two different intermediary
packages contributes identical bytes and unifies to one identity at a consumer.
A dependency type is written only when the package actually names it in an
exported signature; a type reached only through an imported function's own
signature is not re-exported.

Because the executable decode-and-merge above collapses types by their bare name,
a consumer that installs the owning dependency (transitively — it need not be
declared directly) resolves every foreign reference to that one merged
definition. Importing an intermediary therefore brings the re-exported type into
scope under the owning package's original identity (true namespace re-export),
idempotently when several intermediaries surface the same type.
`read_package_type_exports` fills a re-exported type's fields back in from the
owner's sibling `.mfp`, so an importer sees it with its structure intact.

The owning ABI hash is the compatibility gate: when two intermediaries were built
against ABI-incompatible versions of the shared dependency (or an intermediary's
hash disagrees with the owner the consumer resolves), the consumer build is
rejected rather than silently passing an incompatible value between them.
[[src/binary_repr/sections.rs:type_id]] [[src/manifest/package.rs:verify_foreign_type_abi_consistency]]

## MFP Package Container

Package projects emit a `.mfp` file through the package writer.

The package path is:

```text
IR
  -> binary-representation encoding
  -> MFP container wrapping
  -> <package>.mfp
```

Package metadata is derived from `project.json`:

- `name`
- `version`
- `author`
- `url`
- dependency constraints from `packages`

The package writer emits the MFP container carrying its own container version
(major/minor `1.0`) wrapping the inner MFPC `packageBinaryRepr` payload (whose
own container major is `2`). The two version planes are independent: the outer
MFP container format and the inner MFPC binary-representation format version
separately. The exact container header byte fields are documented in
`./mfb spec package container-format`.

Signing is selectable. Without `--sign`, the package writer emits an unsigned
container (signature type 0, zero-length signature); with `--sign <owner>`, the
same writer emits the ed25519-signed form, whose signature covers every header
byte directly and the payload through the embedded payload hash. The reader
accepts both forms; the on-disk signature-header byte encoding is owned by
`./mfb spec package container-format`.[[src/target/package_mfp/mod.rs:build_package_bytes]] [[src/manifest/package.rs:read_mfp_header]]

## Error Source Locations

Every user-visible `Error` carries an `ErrorLoc source` recording where it
originated. The location flows through every layer:

- **AST**: `Expression::Call`/`Binary`/`Unary` and `Statement::For`
  carry an internal `(line, column)`; the source file is the enclosing `AstFile`.
  These are not serialized to the `.ast` JSON.[[src/ast/]]
- **IR**: every `IrOp`, `IrMatchCase`, and declaration node
  (`IrFunction`/`IrParam`/`IrType`/`IrField`/`IrVariant`/`IrBinding`) carries an
  `IrSourceLoc { line, column }`, and computed value nodes carry their result
  type; each `IrFunction` also carries its source `file` and `resource_owners`,
  `IrType`/`IrBinding` carry `file`, and `Bind`/`IrBinding` carry an
  `explicit_type` flag (format v4 — `./mfb spec package ir-section` has the full
  field list). The `error(code, message)` built-in lowers to nested record
  constructors — `Error[code, message, ErrorLoc[file, line, char]]` — so
  `Error`/`ErrorLoc` are ordinary records for the rest of the pipeline. The
  `loc`, result-type, and `explicit_type` fields are not serialized to the
  `.ir` JSON debug dump but **are** encoded into the Binary Representation, so
  an imported package's functions retain their own source locations and stay
  checkable without re-inference.[[src/ir/]]
- **NIR**: mirrors the IR fields (`NirSourceLoc`,
  `NirFunction::file`).[[src/target/shared/nir/]]
- **Native runtime**: the code generator tracks the
  current function file and the current node location and builds a real
  `ErrorLoc` at every error origin (user `error(...)`, arithmetic
  overflow/divide-by-zero, failing built-in/helper calls). The origin is then
  carried through the fallible-call result ABI — owned by
  `./mfb spec memory fallible-call-abi` — and materialized into the 3-field
  `Error` when a result traps.[[src/target/shared/code]]

## See Also

* ./mfb spec memory fallible-call-abi — the four-register result ABI
* ./mfb spec package binary-representation — the on-disk package payload
* ./mfb spec package container-format — the MFP container header and section catalog
* ./mfb spec package doc-section — the documentation-table encoding
* ./mfb spec package ir-section — the package identity hash derivation
* ./mfb spec architecture ir — the in-memory IR this representation serializes
