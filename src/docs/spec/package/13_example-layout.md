# Minimal Example Layout

A small package:

```basic
EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
```

Produces conceptually:

```text
MFPHeader
  magic
  versions (container 1.0)
  name = "mathstuff"
  ident = "ada#mathstuff"
  version = "1.0.0"
  author = "..."
  url = "..."
  identKey = "ed25519:..."          (empty when unsigned)
  signingKey = "ed25519:..."        (one-off key; empty when unsigned)
  proof + proofSig                  (ident-signed; empty when unsigned)
  attestation + attestationSig      (server-signed; empty when unsigned)
  packageBinaryHash = SHA-256(packageBinaryRepr)
  binaryReprLength = N
  signatureType
  signatureLength
  signature                         (by the one-off signing key)

packageBinaryRepr
  BinaryReprHeader
  MANIFEST
  STRING_POOL
    "mathstuff"
    "ada#mathstuff"
    "1.0.0"
    "ed25519:..."
    "<hex ident fingerprint>"
    "<hex signing fingerprint>"
    "add"
    "a"
    "b"
  TYPE_TABLE
    Integer references built-in type id 3
  CONST_POOL
    empty
  IMPORT_TABLE
    empty
  EXPORT_TABLE
    add -> function 0
  ABI_INDEX
    add -> SHA-256 ABI hash
  GLOBAL_TABLE
    empty
  FUNCTION_TABLE
    function 0: add(Integer, Integer) AS Integer  (zero-length code region)
  IR
    "MFBR" + version 4 + IrProject { name, entry?, bindings, types, functions:[ add: Return(Binary{ Add, Ident a, Ident b }) ] }
```

The `IR` payload is self-contained (inline strings); the `TYPE_TABLE`/`EXPORT_TABLE`/`ABI_INDEX`/etc. above are the parallel derived metadata. This package has no `LINK` blocks, so the `MFBR` payload ends after `functions` with no native trailer, and there is no `NATIVE_LINK_TABLE` or `RESOURCE_TABLE` section.

The function body is the structured Binary Representation node for `add`, which decodes back to:

```text
RETURN  ( a + b )
```

i.e. an `IrOp::Return` whose value is an `IrValue::Binary { op: Add, left: a, right: b }`. There are no registers or opcodes — the consumer decodes this back to IR and lowers it through `IR → NIR → native`. If `a + b` overflows at runtime, the checked `Add` produces `ErrOverflow` (`77050010`) and the function returns `Err` (or routes to a `TRAP` region if one encloses it).

## See Also

* ./mfb spec package container-format — the `MFPHeader` shown in the example
* ./mfb spec package binary-representation — the payload section structure illustrated
* ./mfb spec package compact-summary — the condensed table reference
