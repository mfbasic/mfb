# `.mfp` Package Format

A `.mfp` file is a MFBASIC package container, either unsigned or signed with an Ed25519 identity chain. [[wire/src/mfp.rs:validate_signature_header]] It contains:

```text
MFP container header
MFB architecture-independent Binary Representation
```

The container header provides quick package identity and signature information. The **Binary Representation** payload contains the package manifest, dependency metadata, public API metadata, type tables, constants, functions, native binding declarations, and the structured encoding that carries every function body.

The **Binary Representation** is a compact, **versioned, structured** binary encoding of a compiled program. It is *not* a flat register/stack machine: there is no opcode ISA, no `JMP`/`JMP_FALSE`, and no program counter. Control flow stays nested (regions with explicit ends) and expressions stay as trees. A consumer **decodes** the Binary Representation and lowers it through the single codegen path used for the executable's own code, using the same native lowering pipeline. [[src/target/shared/nir/lower.rs:merge_packages]] (The Binary Representation is a versioned external serialization of the compiler's internal program model; see `./mfb spec architecture binary-representation` for how the two relate.)

All integers in `.mfp` files are little-endian. All strings are UTF-8 byte strings and are length-prefixed. No field is NUL-terminated.

## Reading order

The topics below follow the package file from its outer container inward to the
structured payload. `container-format` specifies the signed `.mfp` header,
signature coverage, flags, and container validation. `binary-representation`
describes the architecture-independent payload, its header, and the section
table. `metadata-encoding` covers the string pool, manifest, and the import,
export, and ABI index tables; `type-table`, `constant-pool`, `globals`, and
`functions` specify the remaining metadata sections. `ir-section` describes the
structured Binary Representation that carries every function body, and
`resource-regions` covers how resource lifetime is encoded implicitly by lexical
scope. `native-bindings` specifies how native `LINK` metadata is carried (an interface trailer inside `IR` and library locators in section 10) and the resource table;
`doc-section` covers the optional documentation payload, and `package-meta-section`
specifies the optional package description.
`verifier-rules` distinguishes container and metadata parsing, build-time trust
verification, and merge-time semantic checks. `example-layout` walks a minimal
package end to end, and `compact-summary` is the pasteable short-form spec.

## See Also

* ./mfb spec architecture — how packages are produced, merged, and consumed
* ./mfb spec language modules-and-packages — source-level packages and imports
* ./mfb spec memory — value layouts referenced by the type tables
* ./mfb spec linker — package linking into native executables
* ./mfb spec threading — worker packages
* ./mfb man — built-in package and function help
