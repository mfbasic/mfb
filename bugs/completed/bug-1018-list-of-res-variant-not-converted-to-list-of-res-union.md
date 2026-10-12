# bug-1018: a `List OF RES fs::File` given where a `List OF RES Handle` (a resource union) is expected is accepted unconverted, so `MATCH` takes no arm

Last updated: 2026-10-11
Effort: small (<1h) once bug-999 and bug-1012 are fixed
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs22-agent-g)
Regression Test: (proposed) a resource case in tests/rt-behavior/types/union-container-conversion-rt, plus a `--debug` alloc == free loop

bug-989 (owner decision: a collection of union variants is converted where the union's collection
is expected) covers data unions. The same position with `RES` elements is not converted: the list
is passed as it is, its elements carry no union tag, and `MATCH` matches nothing.

**Correct behavior:** the bug-989 rule applies here too. The list is converted, each handle is
wrapped as the union, `MATCH` takes the `fs::File` arm, the source handle stays open and is
closed once.

**Why it is blocked:** bs18-agent-j implemented the conversion and it was functionally right
(correct arm, source handle open, no double close), but its temporaries were never freed: 102
conversions left 315 allocs / 110 frees / 16,560 B live (`/tmp/bs18j/resl.mfb`, agent-j's
measurement). The causes are bug-999 (an aggregate of resources returned inline is not a temporary)
and bug-1012 (a resource-union box has no owner outside a binding). Land those first, then enable
this conversion and prove `alloc == free`.

## Failing Reproduction

Measured by the coordinator on 2026-10-10, on main's code (`/tmp/main-dbg-target/release/mfb`) and
on bs18-agent-j (bug-989 fixed): the same result on both.

```
IMPORT io
IMPORT fs
IMPORT tcp
IMPORT collections

UNION Handle
  fs::File
  tcp::Socket
END UNION

FUNC describe(hs AS List OF RES Handle) AS Nothing
  FOR EACH h IN hs
    MATCH h
      CASE fs::File(f)
        io::print("file")
      CASE tcp::Socket(s)
        io::print("socket")
      CASE ELSE
        io::print("NO ARM MATCHED")
    END MATCH
  NEXT
END FUNC

FUNC main AS Integer
  MUT files AS List OF RES fs::File = []
  RES f AS fs::File = fs::createTempFile()
  files = collections::append(files, f)
  describe(files)
  RETURN 0
END FUNC
```

- Observed: `NO ARM MATCHED`, exit 0.
- Expected: `file`, exit 0.

## Related

- bug-989 (the data-union form, fixed), bug-999, bug-1012.

## Attempt in bug-staging-21 (bs21-agent-b, 2026-10-11): stopped, nothing shipped

bug-1012 is fixed and bug-999 is fixed for one class (its doc). That is not enough for this bug. The conversion was
enabled, measured at every position, and reverted; the tree is unchanged for this bug.

### The base is worse than the report says

`LET hs AS List OF RES Handle = give(files)`, where `give(files AS List OF RES fs::File) AS List OF RES Handle` is
`RETURN files`, does not print `NO ARM MATCHED`: it dies with SIGSEGV on the base compiler
(`/tmp/bs21-agent-b/m1018/v07.mfb`, `N=1`, exit 139 at the default optimization level and at `-O0`). `mfb dbg`:
`--[XC] F: main+0x24f8 X: a bad memory access at 0x9690550163b2b7dd`. The binding of a call result walks what it
received as `{tag, record}` union elements, and the elements are bare file records. So the unconverted list is a
memory-safety defect at a bind of a call result, not only a `MATCH` that takes no arm.

### The conversion, enabled

Three small edits in `src/ir/lower.rs` (the diff is at the end of this section): `union_collection_converts` and
`union_conversion` compare the element types with the `RES` marker peeled from both sides, and the lifted list
converter takes and answers the bare element (`(value AS fs.File) AS Handle`). With the marker left on, the build
fails with `native collection packed payload does not support type 'RES Handle'`.

Functionally right at every position tried (`/tmp/bs21-agent-b/m1018/v*.mfb`; `files` holds two open temp files,
bound outside the loop; each iteration converts once and `describe` writes to each file through the union):
`MATCH` takes the `fs::File` arm for every element, both source files are still open and written to after the loop,
`double_free_skips 0`, and the bind of a call result above no longer crashes.

It does not balance. alloc / free / live at N=10 and N=40, `--debug` arena 0:

| position | conversion enabled | base (unconverted; `MATCH` takes no arm) |
|---|---|---|
| no conversion, `len(files)` | 13/13/0, 13/13/0 | 13/13/0, 13/13/0 |
| argument, `describe(files)` | 53/22/2960, 173/52/11120 | 13/12/240, 13/12/240 |
| `LET hs AS List OF RES Handle = files` | 53/22/2960, 173/52/11120 | 13/12/240, 13/12/240 |
| `MUT hs = []` then `hs = files` | 63/22/3440, 213/52/13040 | 23/12/720, 53/12/2160 |
| `describe(give(files))`, `give` RETURNing `files` | 53/22/2960, 173/52/11120 | 13/12/240, 13/12/240 |
| constructor field, `Box[files]` and `Box[hs := files]` | 63/22/3120, 213/52/11760 | 23/12/400, 53/12/880 |
| `LET hs AS List OF RES Handle = give(files)` | 53/22/2960, 173/52/11120 | SIGSEGV |
| `Map OF String TO RES fs::File` argument | 83/62/6320, 293/212/24560 | 43/32/3280, 133/92/12400 |

So one list conversion of two elements costs 4 allocations and 1 free, 272 bytes left: the converted list's block
(240) and one 16-byte box per element. (`mfb dbg`, `bp _mfb_arena_alloc bt 3` at N=1: one allocation in `main` for
the output list, two in the lifted converter, one more in `main` that is freed.)

### Why it cannot be balanced inside this bug

1. **The boxes.** The converter's `RETURN` hands `collections::transform` a fresh `{tag, record}` box per element;
   `lower_list_append_in_place` copies its two words into the output and nothing frees the box. bug-1012 gave a box
   an owner in a binding and in a statement, not in a callback result a natively lowered member consumes.
   `lower_transform` frees a callback's flat block result (`free_callback_result`) and has no arm for a
   resource-union box. A user-written `transform` whose callback returns a resource union has the same leak today.
2. **The converted list's block, in an argument.** It is a fresh block of VIEWS: its elements name records the
   source list's owner holds. It is produced by a natively lowered builtin and reaches a resource-union box, which
   is outside bug-999's fixed class on both counts, and it must not be adopted (that would close the source's
   files). It needs a rule bug-999 does not have: free the block, and only the block, of a view aggregate no call
   keeps.
3. **The converted list's block, everywhere else.** At a constructor field the conversion is literally
   `Box[<call returning an aggregate of resources>]`, the position bug-999's document says needs a language
   decision. At a binding, an assignment and a `RETURN` the block's owner is decided by `owning_collections`, which
   takes the strict rule for any collection reaching a resource union (`block_held_elsewhere_strict`): `hs` passed
   to `describe` counts as held elsewhere, so its block is never freed. That is record/collection ownership of
   resource unions, the subject of bug-1025, bug-1028 and bug-1037.

Shipping the conversion as it stands would replace a wrong answer (and one crash) with a correct answer that leaks
272 bytes per two-element conversion. That is the trade the owner declined in bug-989, so it is not shipped.

### The diff that enables the conversion (not applied)

```diff
diff --git a/src/ir/lower.rs b/src/ir/lower.rs
index 296264752..93fabe776 100644
--- a/src/ir/lower.rs
+++ b/src/ir/lower.rs
@@ -6209,6 +6209,11 @@ fn union_conversion(
         ParameterType::Res(inner) => inner.as_ref(),
         other => other,
     };
+    // bug-1018: and the element of a `List OF RES fs::File` is `RES fs::File`.
+    let actual = match (actual, expected) {
+        (ParameterType::Res(inner), ParameterType::Res(_)) => inner.as_ref(),
+        _ => actual,
+    };
     if context
         .type_index
         .variant_belongs_to_union(actual, union_type)
@@ -6228,6 +6233,13 @@ fn union_conversion(
     let loc = context.current_loc;
     match (actual, expected) {
         (ParameterType::ListOf(from), ParameterType::ListOf(to)) => {
+            // bug-1018: the converter takes and answers the element itself; the
+            // `RES` marker is the collection's, as a `RES h AS Handle` binding's
+            // lowered type is the bare `Handle`.
+            let (from, to) = match (from.as_ref(), to.as_ref()) {
+                (ParameterType::Res(from), ParameterType::Res(to)) => (from, to),
+                _ => (from, to),
+            };
             let converter = union_converter(from, to, context);
             IrValue::Call {
                 target: "collections.transform".to_string(),
@@ -6844,6 +6856,13 @@ impl TypeIndex {
         expected: &ParameterType,
     ) -> bool {
         let part = |from: &ParameterType, to: &ParameterType| {
+            // bug-1018: a `RES` element of a variant converts to a `RES` element
+            // of its union as a plain element does; the marker is ownership, on
+            // both sides or on neither.
+            let (from, to) = match (from, to) {
+                (ParameterType::Res(from), ParameterType::Res(to)) => (from.as_ref(), to.as_ref()),
+                other => other,
+            };
             self.variant_belongs_to_union(from, to) || self.union_collection_converts(from, to)
         };
         match (actual, expected) {
```

## Resolution (bs22-agent-g, 2026-10-11)

The general cause is a union rule, not a collection rule: `TypeIndex::variant_belongs_to_union` compared exact spellings, so a variant written with the `RES` alias marker (a list element) or a `STATE` clause was "not a member" and no wrap or conversion was emitted. Membership is asked of the bare types in that one place now, the wrap records the bare variant, and the list converter is handed the element without the slot's marker. Nothing collection-side was written for this bug. It balances because of rule 3 (the converted list is a flat block its statement or binding frees) and because a callback's resource-union result box is freed once the collection has copied its two words (`free_callback_result`).

Measured with `/tmp/bs22/bin/probe.sh` and `/tmp/bs22/sweep.sh` (alloc/free/live bytes, `--debug` arena 0), this branch's compiler:
b1018 prints `file`, 9/9/0. /tmp/bs22-agent-d/src18 v01..v07: `n=20` at N=10 with 53/53/0 or 63/63/0, `n=80` at N=40 balanced, same at -O0, N=300 exits 0 balanced; v08 (`Map`): 83/83/0, 293/293/0, 93/93/0 at -O0. On main these print the no-arm total (`n=200000`) and v07 dies with SIGSEGV.

Tests: `tests/runtime/rt_res_list_from_call_leak.rs` (the list block counts), the 31 resource targets of the batch. No rule keyed on where the call or the value is written.

## Golden impact

Not regenerated. `scripts/artifact-gate.sh <mfb> all`: every fixture that binds, copies, reassigns or drops a collection of `RES` elements gains the ordinary copy and scope-drop of a flat list (`.ncode`), and every call to a function body that returns a value reaching resources gains the statement's take and drain.
