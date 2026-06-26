# Functions

The `FUNCTION_TABLE` stores all functions, native wrapper functions, imported function references, and package initializer functions. The table *describes* each function (name, signature, kind, flags, parameters, declared return/effect); the function *body* is the structured Binary Representation carried in the `IR` section.

```text
functionCount   u32
FunctionEntry[functionCount]
```

## Function entry

```text
name            stringId
kind            u16
flags           u16

paramCount      u32
returnType      typeId

codeOffset      u64
codeLength      u64
```

Because function bodies are carried by the `IR` section as structured Binary Representation, the function table records **zero-length** code regions (`codeOffset`/`codeLength` are retained for layout compatibility and are zero). There are no register tables, program counters, `trapPc`, or cleanup tables in the function entry: those flat-machine concepts do not exist in the structured form. A function's `IF`/`WHILE`/`FOREACH`/`MATCH`/`TRAP` structure, its resource regions, and its single bottom trap are all represented directly as nested Binary Representation nodes.

Function kinds:

```text
1 = binary representation function (structured Binary Representation body)
2 = imported function
3 = native wrapper function
4 = built-in function reference
5 = package initializer
```

Function flags:

```text
bit 0 = exported
bit 1 = private
bit 2 = isolated
bit 3 = sub
bit 5 = returnsNothingOnSuccess
```

The `returnType` is the declared success type. The effective runtime result is always `Result OF returnType`, consistent with the language rule that every function returns `Result` and call sites auto-unwrap or auto-propagate unless directly matched. Whether a function contains a trap is read directly from its Binary Representation body (a `Trap` region), not from a flag/PC pair.

## Parameters

Each function records its parameters (name, type, ownership annotations, default presence and default constant). Parameters with defaults carry the default value; ownership annotations record borrow/consume behavior.

Parameter flags:

```text
bit 0 = has default
bit 1 = resource borrow
bit 2 = resource consume
```

No `BORROW` or `MOVE` source syntax is required. These are compiler/runtime metadata rules, and they round-trip through the Binary Representation.
