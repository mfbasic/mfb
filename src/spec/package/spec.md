# `.mfp` Package Format

A `.mfp` file is a signed MFBASIC package. It contains:

```text
MFP container header
MFB architecture-independent Binary Representation
```

The container header provides quick package identity and signature information. The **Binary Representation** payload contains the package manifest, dependency metadata, public API metadata, type tables, constants, functions, native binding declarations, and the structured encoding that carries every function body.

The **Binary Representation** is a compact, **versioned, structured** binary encoding of a compiled program. It is *not* a flat register/stack machine: there is no opcode ISA, no `JMP`/`JMP_FALSE`, and no program counter. Control flow stays nested (regions with explicit ends) and expressions stay as trees. A consumer **decodes** the Binary Representation and lowers it through the single codegen path used for the executable's own code, so package functions get every language feature for free. (The Binary Representation is a versioned external serialization of the compiler's internal program model; see `architecture.md` for how the two relate.)

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
scope. `native-bindings` specifies how native `LINK` metadata is carried (as a trailer
inside the `IR` payload, not a separate section) and the resource table;
`doc-section` covers the optional documentation payload.
`verifier-rules` lists the container, section, type, function, resource, and
native checks a reader must run before import. `example-layout` walks a minimal
package end to end, and `compact-summary` is the pasteable short-form spec.
