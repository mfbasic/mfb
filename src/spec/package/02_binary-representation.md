# MFB Package Binary Representation

The package binary representation is the architecture-independent payload stored after the `.mfp` header.

The binary representation is not machine code. It contains no native addresses, host pointers, host object layouts, CPU instructions, or platform-specific calling conventions. It is the **structured Binary Representation** — a faithful, versioned serialization of the compiled program — plus the metadata tables that describe the package.

The package container format is called **MFPC**. Its container major version is **2** (the clean break to the structured Binary Representation; the old flat opcode payload was major `1` and is rejected outright).

The Binary Representation is *not* a flat opcode stream: control flow is encoded as nested regions with explicit ends (`IF/THEN/ELSE/END`, `WHILE/DO/END`, `FOREACH/IN/DO/END`, `MATCH/CASE/.../END`, `TRAP/.../END`) and expressions stay as trees (`Binary`, `Call`, `CallResult`, `ResultIsOk/Value/Error`, `Constructor`, `MemberAccess`, literals, identifiers, …). A reader walks the tree; structure is read, never reconstructed from jumps. This is the same principle WebAssembly uses (structured control flow, no arbitrary jumps), kept at MFBASIC's own semantic level so the encoding still knows `List`, `Map`, `Result`, owned `File`, and threads.

```text
packageBinaryRepr
  BinaryReprHeader
  SectionTable
  SectionData...
```

## Binary Representation header

```text
bcMagic        4 bytes
bcMajor        u16   = 2 (structured Binary Representation; major 1 was the old flat payload and is rejected)
bcMinor        u16
bcFlags        u32
sectionCount   u32
sectionTable   SectionHeader[sectionCount]
sectionData    byte[]
```

Recommended `bcMagic`:

```text
4D 46 50 43
M  F  P  C
```

## Section header

```text
sectionId      u16
sectionFlags   u16
reserved       u32
offset         u64
length         u64
```

`offset` is relative to the start of `packageBinaryRepr`, not the start of the file.

Sections may appear in any order, but section ranges must not overlap. Required sections must be present exactly once.

## Section IDs

```text
1  = MANIFEST
2  = STRING_POOL
3  = TYPE_TABLE
4  = CONST_POOL
5  = IMPORT_TABLE
6  = EXPORT_TABLE
7  = GLOBAL_TABLE
8  = FUNCTION_TABLE
10 = NATIVE_LINK_TABLE
11 = RESOURCE_TABLE
12 = DEBUG_INFO
13 = SOURCE_MAP
14 = AUDIT_INFO
15 = ABI_INDEX
16 = IR
17 = DOC
```

Section id `9` (the old flat `CODE` stream) is **retired**. Function bodies are now carried by the `IR` section (id `16`) as structured Binary Representation; the function table records zero-length code regions.

Required sections:

```text
MANIFEST
STRING_POOL
TYPE_TABLE
CONST_POOL
IMPORT_TABLE
EXPORT_TABLE
GLOBAL_TABLE
FUNCTION_TABLE
IR
ABI_INDEX
```

Optional sections:

```text
NATIVE_LINK_TABLE
RESOURCE_TABLE
DEBUG_INFO
SOURCE_MAP
AUDIT_INFO
DOC
```

A package containing `LINK` declarations must include `NATIVE_LINK_TABLE`. If a package contains resource types, including native resources, it must include `RESOURCE_TABLE`. A package with at least one exported `DOC` block (or a `PACKAGE` doc block) includes the optional `DOC` section.
