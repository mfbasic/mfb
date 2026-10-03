# Compact summary

A `.mfp` consists of an outer MFP header followed by an MFPC payload. The package can be unsigned or carry an Ed25519 identity chain. Integers are little-endian; strings are length-prefixed UTF-8. [[wire/src/mfp.rs:validate_signature_header]]

| Layer | Current version | Contents | Canonical layout |
| --- | --- | --- | --- |
| MFP container | 1.0, exact | Identity, trust chain, payload hash, prefix signature | ./mfb spec package container-format |
| MFPC payload | Major 2 | Section directory and metadata sections | ./mfb spec package binary-representation |
| MFBR / IR section | 6, exact | Structured executable declarations and bodies | ./mfb spec package ir-section |
| ABI_INDEX | 1 | Export hashes and dependency requirements | ./mfb spec package metadata-encoding |

The outer header and manifest binary-representation version fields are written as 1.0 and are not validated; they are distinct from the enforced MFPC/MFBR versions. [[src/binary_repr/writer.rs:encode_manifest]] [[src/binary_repr/reader.rs:mfp_binary_repr_payload]] [[wire/src/mfpc.rs:read_section_table]] [[src/ir/binary.rs:BINARY_REPR_VERSION]]

The required sections are `MANIFEST`, `STRING_POOL`, `TYPE_TABLE`, `CONST_POOL`, `IMPORT_TABLE`, `EXPORT_TABLE`, `FUNCTION_TABLE`, `IR`, and `ABI_INDEX`. Optional sections are `GLOBAL_TABLE`, `RESOURCE_TABLE`, `NATIVE_LIBRARY_TABLE`, `DOC`, and `PACKAGE_META`. The producer always emits globals; other optional sections are emitted when non-empty. [[src/binary_repr/reader.rs:read_binary_repr_package]] [[src/binary_repr/writer.rs:encode]]

Function bodies and initializers live in the self-contained MFBR tree. Metadata tables provide the parallel view used to import declarations and check ABI hashes. Native interfaces live in the MFBR trailer; native library locators live in section 10. [[src/ir/binary.rs:encode_project]] [[src/binary_repr/sections.rs:encode_native_library_table]]

The package manager checks trust. The metadata reader checks framing, identity, and table consistency. Decoded package IR is merged with the importing project and semantically checked before native lowering. The exact validation limits are specified separately. [[src/cli/build/packages.rs:classify_installed_package]] [[src/binary_repr/reader.rs:read_binary_repr_package]] [[src/target/shared/nir/lower.rs:merge_packages]]

## See Also

* ./mfb spec package verifier-rules — validation stages and limits
* ./mfb spec package type-table — type IDs and payloads
* ./mfb spec package native-bindings — native interface and resource metadata
* ./mfb spec package doc-section — documentation
* ./mfb spec package package-meta-section — package descriptions
