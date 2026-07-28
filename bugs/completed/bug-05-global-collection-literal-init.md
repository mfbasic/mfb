# bug-05: global collection-literal initializer emits a broken `_mfb_str_empty` reloc

Found: 2026-07-06 (while implementing plan-18-C coverage).

## Symptom

A top-level `MUT`/`LET` binding whose initializer is a **collection literal**
fails to build:

```basic
IMPORT collections
MUT counters AS List OF Integer = [0, 0, 0]     ' or = []
FUNC main AS Integer
  RETURN 0
END FUNC
```

```
error: native code data relocation target '_mfb_str_empty' is not a data object
or defined symbol
```

Both a non-empty (`[0,0,0]`) and an empty (`[]`) list literal reproduce it. The
same literal as a **local** initializer (`MUT r AS List OF Integer = []` inside a
FUNC) works, and a global initialized from a **FUNC call**
(`MUT counters = zeros(3)`) works — so the fault is specific to lowering a
collection *literal* into a global's static/startup initializer, where the
element/empty-string data emission references `_mfb_str_empty` in a context where
that symbol is not a defined data object.

## Scope

Not small-ish: it is a native-codegen fix in the global-binding constant/startup
initialization path (how a collection literal is materialized for a module-level
binding). plan-18-C works around it by initializing its coverage counter global
from a generated FUNC call instead of a literal.

## Repro

`/tmp` scratch project with the snippet above, `mfb build`.

## Fix sketch (unverified)

Lower a global collection-literal initializer the same way a local one is lowered
(runtime construction in the startup sequence), or define the referenced
`_mfb_str_empty` data object in the global-init emission path. Add
`tests/binding_global_list_literal_valid` (build + run) as the proof.
