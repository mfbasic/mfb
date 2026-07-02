# MFB Package Binary Representation

The package binary representation is the architecture-independent payload stored after the `.mfp` header.

The binary representation is not machine code. It contains no native addresses, host pointers, host object layouts, CPU instructions, or platform-specific calling conventions. It is the **structured Binary Representation** — a faithful, versioned serialization of the compiled program — plus the metadata tables that describe the package.

The package container format is called **MFPC**. Its container major version is **2** (the clean break to the structured Binary Representation; the old flat opcode payload was major `1` and is rejected outright). [[src/binary_repr/mod.rs:MFPC_MAJOR_VERSION]]

The Binary Representation is *not* a flat opcode stream: control flow stays nested (regions with explicit ends) and expressions stay as trees, so a reader walks the tree rather than reconstructing it from jumps. That conceptual framing — and why it mirrors WebAssembly's structured control flow at MFBASIC's own semantic level — is owned by `./mfb spec architecture binary-representation`; this page specifies the on-disk byte layout. The concrete node encoding lives in `ir-section`.

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

`offset` is relative to the start of `packageBinaryRepr`, not the start of the file. Each section-table entry is exactly 24 bytes (`sectionId` u16, `sectionFlags` u16, `reserved` u32, `offset` u64, `length` u64), so the table occupies `16 + sectionCount * 24` bytes and section payloads follow.

Sections may appear in any order. The current reader (`read_binary_repr_package`) loads the section table into a map keyed by `sectionId`, validates that each entry's `offset + length` stays within the payload, and then looks sections up by id. Note the gap between the format's intent and the current reader: it does **not** reject overlapping section ranges, and on a duplicate `sectionId` the **last** entry wins rather than being rejected. Producers must still emit each section at most once. [[src/binary_repr/reader.rs:read_binary_repr_package]]

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
10 = NATIVE_LINK_TABLE   (reserved id; not emitted or read — see below)
11 = RESOURCE_TABLE
12 = DEBUG_INFO          (reserved id; not emitted or read)
13 = SOURCE_MAP          (reserved id; not emitted or read)
14 = AUDIT_INFO          (reserved id; not emitted or read)
15 = ABI_INDEX
16 = IR
17 = DOC
```

Section id `9` (the old flat `CODE` stream) is **retired**. Function bodies are now carried by the `IR` section (id `16`) as structured Binary Representation; the function table records zero-length code regions (the `FUNCTION_TABLE` entry format, however, still carries the legacy register/cleanup fields — see `functions`).

Ids `10` (`NATIVE_LINK_TABLE`), `12` (`DEBUG_INFO`), `13` (`SOURCE_MAP`), and `14` (`AUDIT_INFO`) are **reserved by the format but never produced or consumed by the current compiler** — there is no `SECTION_NATIVE_LINK_TABLE` (or debug/source-map/audit) constant in `src/binary_repr/mod.rs`. In particular, native `LINK` metadata is **not** carried in a `NATIVE_LINK_TABLE` section; it rides as an optional trailer inside the `IR` payload (see `native-bindings`).

Sections the current compiler actually emits, via `BinaryReprProject::encode`: [[src/binary_repr/writer.rs:encode]]

```text
MANIFEST          (id 1,  always)
STRING_POOL       (id 2,  always)
TYPE_TABLE        (id 3,  always)
CONST_POOL        (id 4,  always)
IMPORT_TABLE      (id 5,  always)
EXPORT_TABLE      (id 6,  always)
GLOBAL_TABLE      (id 7,  always)
FUNCTION_TABLE    (id 8,  always)
IR                (id 16, always)
ABI_INDEX         (id 15, always)
RESOURCE_TABLE    (id 11, only when the package has resource types)
DOC               (id 17, only when the package has documentation)
```

Sections the current reader (`read_binary_repr_package`) **requires** — rejecting the package if absent:

```text
MANIFEST
STRING_POOL
TYPE_TABLE
CONST_POOL
IMPORT_TABLE
EXPORT_TABLE
FUNCTION_TABLE
IR
ABI_INDEX
```

Sections the reader treats as **optional** (defaulting to empty when absent):

```text
GLOBAL_TABLE      (always emitted by the producer, but tolerated if missing)
RESOURCE_TABLE
DOC
```

A package that contains resource types — including native `LINK` resources — emits `RESOURCE_TABLE`. A package with at least one exported `DOC` block (or a `PACKAGE` doc block) emits the optional `DOC` section.
