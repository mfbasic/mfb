# Functions

The `FUNCTION_TABLE` stores all functions, native wrapper functions, imported function references, and package initializer functions. The table *describes* each function (name, signature, kind, flags, parameters, declared return/effect); the function *body* is the structured Binary Representation carried in the `IR` section.

```text
functionCount   u32
FunctionEntry[functionCount]
```

## Function entry

The function entry retains the **legacy flat-machine layout** from before function bodies moved to the `IR` section. The body-carrying fields are now inert, but the fields themselves are still written and read, so an implementer must account for all of them:

```text
name            stringId
kind            u16
flags           u16
paramCount      u32
returnType      typeId
registerCount   u32      (always 0 for structured functions)
codeOffset      u64      (always 0)
codeLength      u64      (always 0)
sourceMap       u32      (always 0xFFFFFFFF)
cleanupCount    u32      (always 0)
cleanupOffset   u64      (always 0)

repeated paramCount times:        (parameter table)
  paramName     stringId
  paramType     typeId
  paramFlags    u32
  defaultConst  constId or 0xFFFFFFFF

repeated registerCount times:     (register table — currently empty)
  registerType  typeId
  registerFlags u32

repeated cleanupCount times:      (cleanup table — currently empty)
  id                u32
  startPc           u32
  endPc             u32
  resourceRegister  u32
  closeFunctionId   u32
  flags             u32
```

Because function bodies are carried by the `IR` section as structured Binary Representation, the function table records **zero-length** code regions and the producer (`lower_function`) builds every function with an **empty register table and empty cleanup table** (`registers: Vec::new()`, `cleanups: Vec::new()`). The reader rejects any entry whose `codeLength` is non-zero (`flat function code stream is no longer supported`). So while the register/`sourceMap`/cleanup fields are present in the byte layout for compatibility, they are always `0`/`0xFFFFFFFF`/empty in current output: a function's `IF`/`WHILE`/`FOREACH`/`MATCH`/`TRAP` structure, its resource regions, and its single bottom trap are represented directly as nested Binary Representation nodes, not via these tables.

Function kinds:

```text
1 = binary representation function (structured Binary Representation body)
```

Kind `1` (`FUNCTION_BINARY_REPR`) is the **only** kind the current compiler emits — every lowered function, including ones that wrap imported or native targets, is written as kind `1`. [[src/binary_repr/mod.rs:FUNCTION_BINARY_REPR]] Other kind numbers (imported / native wrapper / built-in reference / package initializer) are not produced by the current encoder.

Function flags (u16):

```text
bit 1 = private
bit 2 = isolated
bit 3 = sub
bit 5 = returnsNothingOnSuccess
```

There is **no** "exported" flag bit. A function is exported precisely when it is kind `1` and the private bit is clear (`is_exported_function`); `lower_function` sets the private bit for any non-`export` visibility. [[src/binary_repr/sections.rs:is_exported_function]] A `SUB` sets both the sub bit and `returnsNothingOnSuccess`; a function declared to return `Nothing` also sets `returnsNothingOnSuccess`; an `ISOLATED` function sets the isolated bit. Bit 0 and bit 4 are unused.

The `returnType` is the declared success type. The effective runtime result is always `Result OF returnType`, consistent with the language rule that every function returns `Result` and call sites auto-unwrap or auto-propagate unless directly matched. Whether a function contains a trap is read directly from its Binary Representation body (a `Trap` region), not from a flag/PC pair.

## Parameters

Each parameter records its name, type, `paramFlags`, and a `defaultConst`: a `CONST_POOL` index for a literal default, the `FUNCTION_TABLE` index of the parameter's hidden default function when bit 3 is set, or `0xFFFFFFFF` when the parameter has no default.

Parameter flags (`paramFlags` u32):

```text
bit 0 = has default
bit 1 = resource non-owning  (reserved; not currently emitted)
bit 2 = resource consume     (reserved; not currently emitted)
bit 3 = default function     (defaultConst is a FUNCTION_TABLE index)
```

A default that names nothing (a string, number, scalar or boolean literal, `NOTHING`, or a built-in package constant) is stored as a `CONST_POOL` entry under bit 0 alone. Any other default is **computed**: the compiler lowers it into a private, parameterless hidden default function named `$default$<function>$<index>` that returns the parameter's type (`mfb spec language functions`), and the record sets bits 0 **and** 3 with `defaultConst` holding that function's index. The writer refuses any other default value shape. [[src/binary_repr/writer.rs:lower_param_default]]

A package is untrusted input, and an importer calls the function a bit-3 record names on every call that omits the argument, so the reader refuses — never repairs — a record where bit 3 is set without bit 0, the index is outside the function table, or the target is not private, takes parameters, does not return the parameter's type, or is not named as a hidden default function. [[src/binary_repr/reader.rs:validate_default_functions]] An importer decodes each exported parameter's default (a literal back to its value, or the hidden function's name) and passes it when a call omits the argument. [[src/binary_repr/builder.rs:export_default]]

The non-owning/consume bits are defined by the format but are **not currently produced** — no per-parameter ownership annotation syntax (such as `MOVE`) exists, and resource ownership is enforced by the compiler rather than encoded in these per-parameter bits today.

## See Also

* ./mfb spec package ir-section — where function bodies now live, apart from these describing entries
* ./mfb spec language functions — the source `FUNC`/`SUB` declarations these entries describe
* ./mfb spec package type-table — the signature types entries reference
