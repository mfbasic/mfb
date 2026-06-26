# Globals

The `GLOBAL_TABLE` stores top-level `LET` and `MUT` bindings.

```text
globalCount     u32

repeated globalCount times:
  name          stringId
  type          typeId
  flags         u32
  initFunction  functionId or 0xFFFFFFFF
```

Global flags:

```text
bit 0 = exported
bit 1 = mutable
bit 2 = initialized by constant
bit 3 = initialized by function
```

A package may have a package initializer function. The binary representation merger records package initializers in dependency order so the executable runtime can run them before `main`. Isolated thread package instances run their own package initializers when the thread package instance starts.
