# Globals

The `GLOBAL_TABLE` stores top-level `LET` and `MUT` bindings.

```text
globalCount     u32

repeated globalCount times:
  name          stringId
  type          typeId
  flags         u32
```

Each global entry is exactly three `u32`s. There is **no** per-global `initFunction` field — the current encoding does not store an initializer reference here. A global's initializer expression lives in the IR payload (the binding's value is part of the decoded `IrProject.bindings`), and globals are initialized by running that IR, not by dispatching to a function id named in this table.

Global flags (`flags` u32):

```text
bit 0      = mutable
bits 1-2   = visibility:  0 = private, 1 = package, 2 = export
```

So `MUT` sets bit 0 and `LET` clears it; the visibility two-bit field is `binding.visibility` (`encode`: `private` → `0<<1`, `package` → `1<<1`, `export` → `2<<1`; `decode`: `(flags >> 1) & 0b11`). No "initialized by constant/function" bits exist. There is no separate "exported" bit — a global is exported when its visibility field is `export` (`2`).

Globals are initialized by executing their IR-level initializer. (Cross-package initialization ordering is a merge/runtime concern handled when package IR is merged into the project; it is not encoded as data in this table.)
