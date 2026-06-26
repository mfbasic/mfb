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
  versions
  signatureType
  signatureLength
  signature
  name = "mathstuff"
  ident = "ada#mathstuff"
  version = "1.0.0"
  identKey = "ed25519-public:..."
  identFingerprint = "sha256:..."
  signingFingerprint = "sha256:..."
  author = "..."
  url = "..."
  binaryReprLength = N

packageBinaryRepr
  BinaryReprHeader
  MANIFEST
  STRING_POOL
    "mathstuff"
    "ada#mathstuff"
    "1.0.0"
    "ed25519-public:..."
    "sha256:..."
    "sha256:..."
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
    "MFBR" + version + IrProject { function 0 body: Return(Binary{ Add, Ident a, Ident b }) }
```

The function body is the structured Binary Representation node for `add`, which decodes back to:

```text
RETURN  ( a + b )
```

i.e. an `IrOp::Return` whose value is an `IrValue::Binary { op: Add, left: a, right: b }`. There are no registers or opcodes — the consumer decodes this back to IR and lowers it through `IR → NIR → native`. If `a + b` overflows at runtime, the checked `Add` produces `ErrOverflow` (`77050010`) and the function returns `Err` (or routes to a `TRAP` region if one encloses it).
