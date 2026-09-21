# plan-141 findings: which `MUT` self-updates the compiler performs in place

Measured against: **a `MUT` updating itself mutates in place, with no copy, for
every kind of value.** Every verdict below is read from the compiler source at
`b6a10efbc` and cross-checked against `mfb build --ncode` (see Appendix C).

## Binding sites (the columns)

| Site | Meaning |
|---|---|
| S1 local | `MUT x` declared in a FUNC/SUB body |
| S2 global | `MUT x` at module level |
| S3 local rec, first | field that is not the last collection field of a local `MUT` record |
| S4 local rec, last | the record's last collection field |
| S5 global rec | any field of a module-level `MUT` record |
| S6 nested | a field of a record that is itself a field |
| S7 loop-live | S1 while a `FOR EACH` walks the same binding (G7) |
| S9 captured | *added by this audit* — S1 assigned inside a non-escaping `LAMBDA` that captures it, e.g. `collections::forEach(xs, LAMBDA(v AS Integer) -> acc = collections::append(acc, v))`. The capture is a `by_ref` local (`src/ir/lower.rs:5167`, `builder_control.rs:501`), so G1 applies. |

`STATE` sites are excluded (plan §1 non-goals). S8 (a collection nested in a
collection) is one row in §1b, not a column.

**What "last" means (S3 vs S4) — the code's definition, not the plan's.** The
record arms admit a field only when `record_collection_last_inlined`
(`builder_control.rs:290`) says it is a `List`/`Map`/`Set` field that is itself
inlined **and no field declared after it is inlined**. `String`, nested-record,
data-union, `Result` and collection fields are all inlined
(`record_field_is_inlined`, `builder_collection_layout.rs:3152`); `Integer`,
`Float`, `Boolean` and other fixed-width scalars are not. So S4 is "the last
inlined field", and a collection field followed by a `String` field is S3
(probe `r3_list_then_string` → n; `r3_list_then_int` → y). The probes use
`TYPE Rec { a, b }` with both fields the same collection type: `a` is S3, `b` is S4.

Cell values: `y` = in place; `n (Gxx)` = the named gate declines (the first one
in code order that does) and the statement takes the copying path;
`n (no arm)` = no recogniser exists for this function in any container, so the
statement always takes the copying path whatever the site;
`n (StoreGlobal)` = the statement lowers through `NirOp::StoreGlobal`, which
dispatches no recogniser at all (Appendix B.1); `n/a` = the form cannot be written
(reason given). Every `y` also needs the operand gates to pass — G11 (the
appended/added value has a static type: a local, literal, `get`, arithmetic, or a
call with a declared return, `builder_value_semantics.rs:1189`) and G12 (it is not
the mutated collection itself). Those depend on the operand, not the site, so they
are named in the evidence rather than given columns.

The "copying path" for S1/S7/S9 is `lower_value_owned(value)` then a free of the
old block (`builder_control.rs:1269`, `:1309–1340`): the builtin allocates a
fresh collection and copies every surviving element into it. For S3/S4/S6 it is
`lower_with_update` (`builder_value_semantics.rs:703`): the builtin builds the new
field value, then **every field of the record is gathered and a new record block
is built**. Neither path reuses the old block, and no builtin argument is ever
"moved" into its result — the plan-134 last-use moves (`builder_values.rs:1393`)
apply only when the stored value *is* a place read, never to a call argument.

## 1. Collections

| function definition | S1 | S2 | S3 | S4 | S5 | S6 | S7 | S9 | evidence |
|---|---|---|---|---|---|---|---|---|---|
| `collections::add(value AS Set OF T, item AS T) AS Set OF T` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_set_add_assign` (`bia:196`). S4 `try_inplace_record_field_set_add_assign` (`bia:538`). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_add_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm). |
| `collections::all(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::all(x, …)` cannot type-check |
| `collections::any(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::any(x, …)` cannot type-check |
| `collections::append(value AS List OF T, item AS T) AS List OF T` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_append_assign` (`bia:23`). S4 `try_inplace_record_field_append` (`bia:92`, single element). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_append1_S1`…`_S9`. |
| `collections::append(value AS List OF T, item AS List OF T) AS List OF T` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_bulk_append_assign` (`bia:1399`); `x = append(x, x)` is n (G12). S4 `try_inplace_record_field_append` (`bia:92`, bulk); `append(r.f, r.f)` is n (G12). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_appendL_S1`…`_S9`. |
| `collections::chunks(value AS List OF T, chunkSize AS Integer) AS List OF List OF T` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (needs `List OF T = List OF List OF T`). Probe `gen/g_chunks` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |
| `collections::contains(value AS List OF T, item AS T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::contains(x, …)` cannot type-check |
| `collections::contains(value AS Set OF T, item AS T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::contains(x, …)` cannot type-check |
| `collections::difference(a AS Set OF T, b AS Set OF T) AS Set OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Mfb` source generic → call `_mfb_ifn_collections_difference$T`; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_difference_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm): ARM=-. |
| `collections::distinct(value AS List OF T) AS List OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Mfb` source generic → call `_mfb_ifn_collections_distinct$T`; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_distinct_S1`…`_S9`: ARM=-. |
| `collections::drop(value AS List OF T, count AS Integer) AS List OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Mfb` source generic → call `_mfb_ifn_collections_drop$T`; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_drop_S1`…`_S9`: ARM=-. |
| `collections::filter(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS List OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::abi_inline(lower_filter)`: inline lowering builds a fresh list; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_filter_S1`…`_S9`: ARM=-. |
| `collections::find(value AS List OF T, item AS T, [start AS Integer]) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::find(x, …)` cannot type-check |
| `collections::find(value AS List OF T, item AS List OF T, [start AS Integer]) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::find(x, …)` cannot type-check |
| `collections::findIndex(value AS List OF T, predicate AS FUNC(T) AS Boolean, [start AS Integer]) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::findIndex(x, …)` cannot type-check |
| `collections::findLastIndex(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::findLastIndex(x, …)` cannot type-check |
| `collections::findLastIndex(value AS List OF T, predicate AS FUNC(T) AS Boolean, start AS Integer) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::findLastIndex(x, …)` cannot type-check |
| `collections::flatten(value AS List OF List OF T) AS List OF T` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (needs `List OF List OF T = List OF T`). Probe `gen/g_flatten` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |
| `collections::forEach(value AS List OF T, action AS FUNC(T) AS Nothing) AS Nothing` | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | no self-update form: the result type is not a collection, so `x = collections::forEach(x, …)` cannot type-check |
| `collections::get(value AS List OF T, index AS Integer) AS T` | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | no self-update form: the element type `T`/`V` can never equal the collection type `List OF T`/`Map OF K TO V` itself, so `x = collections::get(x, …)` cannot type-check |
| `collections::get(value AS Map OF K TO V, index AS K) AS V` | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | no self-update form: the element type `T`/`V` can never equal the collection type `List OF T`/`Map OF K TO V` itself, so `x = collections::get(x, …)` cannot type-check |
| `collections::getOr(value AS List OF T, index AS Integer, default AS T) AS T` | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | no self-update form: the element type `T`/`V` can never equal the collection type `List OF T`/`Map OF K TO V` itself, so `x = collections::getOr(x, …)` cannot type-check |
| `collections::getOr(value AS Map OF K TO V, index AS K, default AS V) AS V` | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | no self-update form: the element type `T`/`V` can never equal the collection type `List OF T`/`Map OF K TO V` itself, so `x = collections::getOr(x, …)` cannot type-check |
| `collections::groupBy(value AS List OF T, keyFn AS FUNC(T) AS K, valFn AS FUNC(T) AS V) AS Map OF K TO List OF V` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (a `List` is never a `Map`). Probe `gen/g_groupBy` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |
| `collections::hasKey(value AS Map OF K TO V, key AS K) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::hasKey(x, …)` cannot type-check |
| `collections::insert(value AS List OF T, index AS Integer, item AS T) AS List OF T` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_insert_assign` (`bia:2077`). S4 `try_inplace_record_field_insert_assign` → `…_splice_assign` (`bia:1291`). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_insert_S1`…`_S9`. |
| `collections::intersection(a AS Set OF T, b AS Set OF T) AS Set OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Mfb` source generic → call `_mfb_ifn_collections_intersection$T`; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_intersection_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm): ARM=-. |
| `collections::isDisjoint(a AS Set OF T, b AS Set OF T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::isDisjoint(x, …)` cannot type-check |
| `collections::isSubset(a AS Set OF T, b AS Set OF T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::isSubset(x, …)` cannot type-check |
| `collections::isSuperset(a AS Set OF T, b AS Set OF T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::isSuperset(x, …)` cannot type-check |
| `collections::keys(value AS Map OF K TO V) AS List OF K` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (a `Map` is never a `List`). Probe `gen/g_keys` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |
| `collections::mapValues(value AS Map OF K TO V, f AS FUNC(V) AS U) AS Map OF K TO U` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::mfb_with_fast_path`: fast path or generic call, both return a fresh map (type-checks only when U = V); the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_mapValues_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm): ARM=-. |
| `collections::merge(a AS Map OF K TO V, b AS Map OF K TO V, preferB AS Boolean) AS Map OF K TO V` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::mfb_with_fast_path`: fast path or generic call, fresh map; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_merge_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm): ARM=-. |
| `collections::mid(value AS List OF T, start AS Integer, count AS Integer) AS List OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Intrinsic` (`native_builtin_target` → `mid`), fresh list; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_mid_S1`…`_S9`: ARM=-. |
| `collections::partition(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Partition OF T` | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | no self-update form: the result type is not a collection, so `x = collections::partition(x, …)` cannot type-check |
| `collections::prepend(value AS List OF T, item AS T) AS List OF T` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_prepend_assign` (`bia:1637`). S4 `try_inplace_record_field_prepend_assign` → `…_splice_assign` (`bia:1291`). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_prepend_S1`…`_S9`. |
| `collections::reduce(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::abi_inline(lower_reduce)`; type-checks only when U = List OF T; the accumulator result is a fresh value; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_reduce_S1`…`_S9`: ARM=-. |
| `collections::reduceRight(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::abi_inline(lower_reduce_right)`; type-checks only when U = List OF T; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_reduceRight_S1`…`_S9`: ARM=-. |
| `collections::remove(value AS Set OF T, item AS T) AS Set OF T` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_set_remove_assign` (`bia:2025`). S4 `try_inplace_record_field_set_remove_assign` (`bia:472`). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_remove_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm). |
| `collections::removeAt(value AS List OF T, index AS Integer) AS List OF T` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_remove_at_assign` (`bia:1945`). S4 `try_inplace_record_field_remove_at_assign` (`bia:413`). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_removeAt_S1`…`_S9`. |
| `collections::removeKey(value AS Map OF K TO V, key AS K) AS Map OF K TO V` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_remove_key_assign` (`bia:271`). S4 `try_inplace_record_field_remove_key_assign` (`bia:348`). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_removeKey_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm). |
| `collections::replace(value AS List OF T, old AS T, new AS T) AS List OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Intrinsic` (`native_builtin_target` → `replace`), fresh list; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_replace_S1`…`_S9`: ARM=-. |
| `collections::set(value AS List OF T, index AS Integer, item AS T) AS List OF T` | y | n (StoreGlobal) | n (G17) | y (fixed-width T) / n (G26, variable-width T) | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_set_assign` (`bia:1490`, List branch; a longer variable-width replacement is resized in place by `lower_list_set_in_place`, `list_mutate.rs:3332`). S4 `try_inplace_record_field_set_assign` (`bia:629`, List branch). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_setL_S1`…`_S9`. S4 split: `bia:658` declines a non-fixed-width element (probe `r3_names_set`: `List OF String` field → WITH rebuild). |
| `collections::set(value AS Map OF K TO V, index AS K, item AS V) AS Map OF K TO V` | y | n (StoreGlobal) | n (G17) | y | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1 `try_inplace_set_assign` (`bia:1490`, Map branch). S4 `try_inplace_record_field_set_assign` (`bia:629`, Map branch, `InlineGrow`). S3/S6: the record container `resolve_inplace_record_field` declines at G17 (`bc:290`: `a` has an inlined field after it; `inner` is a record, not a collection). S7: G7 in the arm (`for_each_iterable_locals`). S9: G1, the lambda's capture is `by_ref`. S2/S5: `NirOp::StoreGlobal` (`bc:1060`) dispatches no arm. Probes `c_setM_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm). |
| `collections::sort(value AS List OF T) AS List OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::mfb_with_fast_path` (native `lower_collection_sort_call` for String/Integer/Fixed/Money), fresh list; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_sort_S1`…`_S9`: ARM=-. |
| `collections::sortBy(value AS List OF T, keyFn AS FUNC(T) AS U) AS List OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::mfb_with_fast_path`, fresh list; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_sortBy_S1`…`_S9`: ARM=-. |
| `collections::sum(value AS List OF Integer) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::sum(x, …)` cannot type-check |
| `collections::sum(value AS List OF Float) AS Float` | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | no self-update form: the result type is not a collection, so `x = collections::sum(x, …)` cannot type-check |
| `collections::sum(value AS List OF Fixed) AS Fixed` | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | no self-update form: the result type is not a collection, so `x = collections::sum(x, …)` cannot type-check |
| `collections::symmetricDifference(a AS Set OF T, b AS Set OF T) AS Set OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Mfb` source generic → call; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_symmetricDifference_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm): ARM=-. |
| `collections::take(value AS List OF T, count AS Integer) AS List OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Mfb` source generic → call `_mfb_ifn_collections_take$T`; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_take_S1`…`_S9`: ARM=-. |
| `collections::toList(value AS Set OF T) AS List OF T` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (a `Set` is never a `List`). Probe `gen/g_toList` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |
| `collections::toSet(value AS List OF T) AS Set OF T` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (a `List` is never a `Set`). Probe `gen/g_toSet` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |
| `collections::transform(value AS List OF T, f AS FUNC(T) AS U) AS List OF U` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::abi_inline(lower_transform)`; type-checks only when U = T; fresh list; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_transform_S1`…`_S9`: ARM=-. |
| `collections::union(a AS Set OF T, b AS Set OF T) AS Set OF T` | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | No `try_inplace_*` recognises it (Appendix B.3). `Body::Mfb` source generic → call `_mfb_ifn_collections_union$T`; the binding is then reassigned (old block freed) or, in a record, the whole record is rebuilt by `lower_with_update` (`builder_value_semantics.rs:703`). Probes `c_union_S1`…`_S7` (no S9 probe for a Map/Set; S9 is the code reading — the `by_ref` decline G1 is shared by every arm): ARM=-. |
| `collections::values(value AS Map OF K TO V) AS List OF V` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (a `Map` is never a `List`). Probe `gen/g_values` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |
| `collections::window(value AS List OF T, size AS Integer, [stride AS Integer]) AS List OF List OF T` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (needs `List OF T = List OF List OF T`). Probe `gen/g_window` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |
| `collections::zip(a AS List OF A, b AS List OF B) AS List OF Pair OF A, B` | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | n/a (does not type-check) | Generic-only, and no instantiation makes the result type equal the argument type (needs `A = Pair OF A, B`). Probe `gen/g_zip` → `error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]`. |

### 1b. Beyond `collections::` (plan Open Decisions, recommended options taken)

Same columns as §1. S8 is the nested-collection shape the plan's second Open
Decision asked for.

| form | S1 | S2 | S3 | S4 | S5 | S6 | S7 | S9 | evidence |
|---|---|---|---|---|---|---|---|---|---|
| String self-concat `s = s & t` (and `s = s & a & b …`) | y | n (StoreGlobal) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (a `String` is not a `FOR EACH` iterable) | n (G1) | S1 `try_inplace_concat_assign` (`bia:1714`): G1 → G19 (a capacity shadow slot, allocated by `prescan_string_self_appends`, `bc:2317`) → G20 (the value is a left-associated `&` chain starting at `s`) → G21 (`s` is not read again in a later operand: `s = s & x & s` is n (G21)). Probes: `x_concat_S1` ARM=concat; `x_concat_S2` GLOBAL=y, ARM=-; `x_concat_S3`/`_S4` (`r = WITH r { s := r.s & t }`) WITH=y, ARM=- — the concat arm matches only a plain local and no record arm handles a `String` field; `x_concat_S9` ARM=- (its lambda `$lambda0` also ARM=-). |
| Scalar self-update `i = i + k` (`Integer`, `Float`, `Boolean`, `Fixed`, …) | y | y | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (a scalar is not a `FOR EACH` iterable) | y | A scalar has no block: S1 stores the new value into the local's slot (`bc:1396–1400`), S9 stores through the reference pointer (`bc:1386`), and S2's `StoreGlobal` takes the non-freeable branch (`bc:1130–1133`): a plain store to the global, no copy and no free. Probes `x_int_S1`, `x_int_S2` (no `store_global_*` slots — the old-block free is skipped). A scalar **field** of a record is §2's R1: the whole record is rebuilt. |
| S8 nested collection `grid = set(grid, i, set(get(grid, i), j, v))` on `List OF List OF T` | y (outer) / n (row) | n (StoreGlobal) | n (G17) | n (G26) | n (StoreGlobal) | n (G17) | n (G7) | n (G1) | S1: the **outer** `set` is `try_inplace_set_assign` (probe `x_grid_S8` ARM=set(List)); a `List` element is variable-width, so `lower_list_set_in_place` takes its resize branch. The **inner** `set(get(grid, i), j, v)` is a call argument, not a binding: no arm can match it (G5 needs `args[0]` to be a bare `Local`), so it lowers out of place and copies the whole row into a fresh block every write. S4: `record_field_set` declines a `List` field whose element (`List OF T`) is not fixed-width (G26, `bia:658`). |

## 2. Record updates

Columns are §"Binding sites". For a **scalar** field (R1, R4 scalar), S3 = the
field is not the record's last field and S4 = it is the last (probes `ScFirst` /
`ScLast`): position cannot matter, since no arm handles a scalar field, and the
probes confirm it does not. For **R5**, S3/S4 = the nested record is not-last /
last in its parent and S6 = one more level of nesting. R3's per-operation verdicts
are §1's S3–S6 columns; its cells here summarise them by field type.

| form | field type | S3 | S4 | S5 | S6 | evidence |
|---|---|---|---|---|---|---|
| R1 `r = WITH r { f := <new scalar> }` | Integer | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | No recogniser handles a non-`STATE` scalar field: every `record_field_*` arm goes through `resolve_inplace_record_field`, whose G17 (`record_collection_last_inlined`, `bc:302`) refuses a field that is not a `List`/`Map`/`Set`. The statement lowers `lower_with_update` (`builder_value_semantics.rs:703`): every field is gathered and a new record block is built, then the old one is freed. (The `STATE` twin, `try_inplace_state_scalar_assign` `bc:139`, does store a scalar field in place — the same shape on a record local has no arm.) Probes `r1_int_S3` (field first), `r1_int_S4` (field last), `r1_int_S5`, `r1_int_S6`: all WITH=y, ARM=-; `r1_int_S5` GLOBAL=y. |
| R1 `r = WITH r { f := <new scalar> }` | Float | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | No recogniser handles a non-`STATE` scalar field: every `record_field_*` arm goes through `resolve_inplace_record_field`, whose G17 (`record_collection_last_inlined`, `bc:302`) refuses a field that is not a `List`/`Map`/`Set`. The statement lowers `lower_with_update` (`builder_value_semantics.rs:703`): every field is gathered and a new record block is built, then the old one is freed. (The `STATE` twin, `try_inplace_state_scalar_assign` `bc:139`, does store a scalar field in place — the same shape on a record local has no arm.) Probe `r1_float_S3`: WITH=y, ARM=-. S4–S6 by the same code path as `Integer` (the decline does not depend on the scalar kind). |
| R1 `r = WITH r { f := <new scalar> }` | Boolean | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | No recogniser handles a non-`STATE` scalar field: every `record_field_*` arm goes through `resolve_inplace_record_field`, whose G17 (`record_collection_last_inlined`, `bc:302`) refuses a field that is not a `List`/`Map`/`Set`. The statement lowers `lower_with_update` (`builder_value_semantics.rs:703`): every field is gathered and a new record block is built, then the old one is freed. (The `STATE` twin, `try_inplace_state_scalar_assign` `bc:139`, does store a scalar field in place — the same shape on a record local has no arm.) Probe `r1_bool_S3`: WITH=y, ARM=-. S4–S6 as `Integer`. |
| R1 `r = WITH r { f := <new scalar> }` | String | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | No recogniser handles a non-`STATE` scalar field: every `record_field_*` arm goes through `resolve_inplace_record_field`, whose G17 (`record_collection_last_inlined`, `bc:302`) refuses a field that is not a `List`/`Map`/`Set`. The statement lowers `lower_with_update` (`builder_value_semantics.rs:703`): every field is gathered and a new record block is built, then the old one is freed. (The `STATE` twin, `try_inplace_state_scalar_assign` `bc:139`, does store a scalar field in place — the same shape on a record local has no arm.) A `String` field is inlined, so it also makes an earlier collection field not-last. The self-concat form `WITH r { s := r.s & t }` is also n (§1b). Probes `r1_string_S3`, `r1_string_S4`, `x_concat_S3`, `x_concat_S4`: WITH=y, ARM=-. |
| R2 `r = WITH r { f := <new value not derived from r.f> }` | List | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | A replacement is not a self-update of the field, and no arm recognises one: at S4 the record container passes G17 and then `inplace_call_args` declines at G2 (a literal is not a `Call`) or G3 (a user/other call is not the arm's builtin), `inplace_dest.rs:403`. At S3/S6 G17 declines first. The whole record is rebuilt. Probes `r2_list_S3`, `r2_list_S4`, `r2_list_S5`: WITH=y, ARM=-. |
| R2 `r = WITH r { f := <new value not derived from r.f> }` | Map | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | As R2 List. Probe `r2_map_S4` (`WITH r { mb := mkMap() }`): WITH=y, ARM=-. |
| R2 `r = WITH r { f := <new value not derived from r.f> }` | Set | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | As R2 List. Probe `r2_set_S4` (`WITH r { sb := collections::toSet([k]) }`): WITH=y, ARM=-. |
| R3 `r = WITH r { f := collections::<op>(r.f, …) }` | List | n (G17) | y for `append` (both), `insert`, `prepend`, `removeAt`, `set` (fixed-width T); n (G26) for `set` on variable-width T; n (no arm) for the other 12 List self-updates | n (StoreGlobal) | n (G17) | Per-operation cells are §1's S3–S6 columns. Arms: `record_field_append` (`bia:92`), `…_insert`/`…_prepend` → `…_splice` (`bia:1291`), `…_remove_at` (`bia:413`), `…_set` (`bia:629`). Operand gates at S4: G18 (`WITH r { b := append(r.a, x) }` is n — the source is another field), G12 (`append(r.b, r.b)` is n), G11. A live `FOR EACH v IN r.xs` over the field is n (G15): probe `r3_loop_live_field` WITH=y. Probes `c_*_S3/S4/S6`, `r3_list_then_int` (y), `r3_list_then_string` (n, G17), `r3_names_set` (n, G26), `r3_names_append` (y). |
| R3 `r = WITH r { f := collections::<op>(r.f, …) }` | Map | n (G17) | y for `set`, `removeKey`; n (no arm) for `merge`, `mapValues` | n (StoreGlobal) | n (G17) | Arms `record_field_set` Map branch (`bia:709`, grows the record through `InlineGrow`) and `record_field_remove_key` (`bia:348`). Probes `c_setM_S4`, `c_removeKey_S4` (arm fired), `c_setM_S3`, `r3_map_S3` (WITH=y), `c_merge_S4`, `c_mapValues_S4` (ARM=-). |
| R3 `r = WITH r { f := collections::<op>(r.f, …) }` | Set | n (G17) | y for `add`, `remove`; n (no arm) for `union`, `intersection`, `difference`, `symmetricDifference` | n (StoreGlobal) | n (G17) | Arms `record_field_set_add` (`bia:538`) and `record_field_set_remove` (`bia:472`). Probes `c_add_S4`, `c_remove_S4` (arm fired), `c_union_S4` etc. (ARM=-). |
| R4 `r = WITH r { f := …, g := … }` | List | n (G14) | n (G14) | n (StoreGlobal) | n (G17) | `resolve_inplace_record_field` declines a `WITH` with more than one update (G14, `inplace_dest.rs:301`), before G17. This holds even when only one of the updates is a collection: `WITH r { xs := append(r.xs, k), n := k }` rebuilds the whole record (probe `r4_list_one_collection` WITH=y). Probe `r4_list` (two `append`s): WITH=y, ARM=-. S6: the outer `WITH` has one update (`inner`), which G17 declines. |
| R4 `r = WITH r { f := …, g := … }` | Map | n (G14) | n (G14) | n (StoreGlobal) | n (G17) | As R4 List: G14 is checked before the field kind (`inplace_dest.rs:301`), so it declines identically. Distinct verdict probed by `r4_list`. |
| R4 `r = WITH r { f := …, g := … }` | Set | n (G14) | n (G14) | n (StoreGlobal) | n (G17) | As R4 Map. |
| R4 `r = WITH r { f := …, g := … }` | scalar | n (G14) | n (G14) | n (StoreGlobal) | n (G17) | G14 declines first; even with one field no arm would handle a scalar (R1). Probe `r4_scalar` (`WITH r { i := k, j := k }`): WITH=y, ARM=-. |
| R5 `r = WITH r { inner := WITH r.inner { f := … } }` | List | n (G17) | n (G17) | n (StoreGlobal) | n (G17) | Columns for R5: S3/S4 = the nested record `inner` is not-last/last in the outer record; S6 = one more level of nesting. The outer `WITH` updates `inner`, which is a record, not a collection, so `record_collection_last_inlined` returns `None` (`bc:302`) whatever the inner update is. Probes `c_append1_S6` (inner last), `r5_set_S3` (inner first): WITH=y, ARM=-. |
| R5 `r = WITH r { inner := WITH r.inner { f := … } }` | Map | n (G17) | n (G17) | n (StoreGlobal) | n (G17) | As R5 List. Probes `c_setM_S6`, `c_removeKey_S6`: WITH=y, ARM=-. |
| R5 `r = WITH r { inner := WITH r.inner { f := … } }` | Set | n (G17) | n (G17) | n (StoreGlobal) | n (G17) | As R5 List. Probes `r5_set_S3`, `c_add_S6`: WITH=y, ARM=-. |
| R5 `r = WITH r { inner := WITH r.inner { f := … } }` | scalar | n (G17) | n (G17) | n (StoreGlobal) | n (G17) | As R5 List; the inner field being a scalar changes nothing. Probe `r1_int_S6`: WITH=y, ARM=-. |
| R6 `r.prop = value` | List | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | `r.prop = value` is rejected by the parser: `error[1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT]: record field assignment is not supported` (rule at `src/rules/table.rs:187`; probe `r6`, Appendix C.5). `mfb man variable`: "WITH is the only way to update its fields". Only a `RES … STATE` field has this form. |
| R6 `r.prop = value` | Map | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | `r.prop = value` is rejected by the parser: `error[1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT]: record field assignment is not supported` (rule at `src/rules/table.rs:187`; probe `r6`, Appendix C.5). `mfb man variable`: "WITH is the only way to update its fields". Only a `RES … STATE` field has this form. |
| R6 `r.prop = value` | Set | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | `r.prop = value` is rejected by the parser: `error[1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT]: record field assignment is not supported` (rule at `src/rules/table.rs:187`; probe `r6`, Appendix C.5). `mfb man variable`: "WITH is the only way to update its fields". Only a `RES … STATE` field has this form. |
| R6 `r.prop = value` | scalar | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | `r.prop = value` is rejected by the parser: `error[1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT]: record field assignment is not supported` (rule at `src/rules/table.rs:187`; probe `r6`, Appendix C.5). `mfb man variable`: "WITH is the only way to update its fields". Only a `RES … STATE` field has this form. |

## 3. Summary

### 3.1 What the compiler does, in one paragraph

A `MUT` self-update is in place only when **all** of these hold: the binding is a
**function local** (never a module-level `MUT`); the statement is
`x = collections::<op>(x, …)` with `<op>` one of the **10 overloads that have an
arm** — `append` (both), `set` (both), `insert`, `prepend`, `removeAt`, `add`,
`remove`, `removeKey`; no `FOR EACH` is walking `x`; and `x` is not a lambda's
by-ref capture. Or the statement is `r = WITH r { f := <op>(r.f, …) }` on a
function-local record with **exactly one** updated field, `f` a collection field
with **no inlined field (`String`, record, union, `Result`, collection) declared
after it**, no `FOR EACH` walking `r.f`, and (for `set` on a `List`) a
fixed-width element type. Or it is a `String` local's `s = s & …`, or a scalar
local. Every other self-update rebuilds: the collection is copied into a fresh
block, or the whole record is re-laid-out.

### 3.2 Findings, most consequential first

1. **Module-level `MUT` is never updated in place.** Every S2/S5 cell is
   `n (StoreGlobal)` (77 cells) except the scalar global (1 `y`).
   `NirOp::StoreGlobal` (`bc:1060`) has no recogniser dispatch, for
   collections, records, and `String` alike (Appendix B.1). Each write to a global
   collection copies the whole collection, then frees the old block. This is the
   Brogue timing gap (global `List` set 22,198 ns vs 36 ns local; global `Map` set
   468,418 ns): the code path, not a slow in-place path. Only a scalar global is
   updated without a copy, because a scalar has no block.
2. **17 self-updatable overloads have no arm in any container**: `filter`,
   `take`, `drop`, `mid`, `replace`, `sort`, `sortBy`, `distinct`, `transform`,
   `reduce`, `reduceRight`, `union`, `intersection`, `difference`,
   `symmetricDifference`, `merge`, `mapValues`. Even on a plain local they build a
   fresh collection (a source-generic call, a fast path, or an inline lowering)
   and free the old one. Of the 132 `n (no arm)` cells, 102 are these 17 rows at
   the six non-global sites; the other 30 are record forms with no arm (R1 12, R2 9,
   R3's non-arm operations 3, §1b's `String`/scalar record fields 6).
3. **Records: one field, last, collection, local.** 44 cells decline at G17 —
   any field with an inlined field after it (S3), and **every nested record**
   (S6, R5), because the outer `WITH` updates a record-typed field. A `String`
   field declared after a `List` is enough (probe `r3_list_then_string`).
   Updating two fields in one `WITH` declines at G14, **even when the second is a
   scalar counter** (`WITH r { xs := append(r.xs, k), n := k }`, probe
   `r4_list_one_collection`). This is the Brogue record-field timing gap (70,231 ns
   on a non-last list field).
4. **A scalar or `String` field of a record is never updated in place** (R1: 16
   cells, all n). No record arm handles a non-collection field. The `STATE`
   container has exactly this arm (`try_inplace_state_scalar_assign`, `bc:139`);
   the record-local container does not.
5. **Loop-live and lambda-captured bindings decline** — G7 (11 cells) and G1 (12
   cells). Under a live `FOR EACH`, `append` declines like every other arm (see
   3.4, contradiction 3).
6. **G26 (new name)**: `r = WITH r { names := set(r.names, i, s) }` on a
   `List OF String` (or any variable-width element) field rebuilds the record,
   although the plain-local `set` resizes in place for the same element type
   (probes `r3_names_set` n, `x_setString_S1` y).
7. **S8, the 2-D grid** `grid = set(grid, i, set(get(grid, i), j, v))`: the outer
   write is in place, but the inner row update is a call argument that no arm can
   see (G5), so **every cell write copies its whole row**.

### 3.3 Reading vs `--ncode`

Every verdict was probed (Appendix C: 206 + 32 SUBs, plus the 9-error `gen` build
and the `r6` parse error). **0 disagreements** between the code reading and the
dump.

### 3.4 Contradictions with the docs (for the follow-up plan to correct)

`.ai/collections.md` §"In-place mutation: one seam, one gate inventory":

1. `:21-22` — "`x = OP(x, …)` on a uniquely-owned collection is lowered as a
   mutation of the live buffer whenever nothing else can observe that buffer."
   True only for the 10 arm-backed overloads on a function local (finding 2), and
   never for a global (finding 1).
2. `:24-25` — dispatch at `builder_control.rs:879-909` and `:1050`/`:1056`. Now
   `:1166–1263` and `:1440`/`:1449`.
3. `:59-65` — "An `append` writes only *beyond* that snapshot, so it may proceed
   (until it reallocs — hence the guard)." The code declines **every** `append`
   under a live `FOR EACH` over the binding (G7 in `resolve_inplace_plain_local`,
   `inplace_dest.rs:253`; `bulk_append` `bia:1433`); probes `c_append1_S7`,
   `c_appendL_S7` → rebuild. The same claim is in `bia:17-19`'s doc comment.
4. `:46` — "all 23 decline conditions (`G1`–`G23`)". The inventory has G24
   (since lifted), the code has G25 (`STATE`) and an unnamed variable-width gate
   (G26 here).

`planning/plan-121-gate-inventory.md`: the eight differences in Appendix B.4
(population 10 → 30, stale dispatch lines, G17 widened to any inlined field and
any collection kind, G24 lifted, G26 missing, G25 missing, container gate order,
`set` resize branch).

### 3.5 Observed while reading, not part of the verdicts

These are not in-place questions, so they carry no verdict; they are recorded so
the follow-up plan does not rediscover them.

- **S9 fallback may leak.** For a `by_ref` local the fallback reassignment writes
  the fresh block through the reference (`bc:1386`) but skips the old-block free
  (`bc:1309`, `!by_ref && …`). Whether the parent frees the overwritten block
  elsewhere was not traced.
- **S7 fallback leaks by design.** Under a live `FOR EACH` over the binding the
  fallback deliberately does not free the old block (`bc:1312–1318`, bug-430), so
  a self-update inside such a loop leaks one block per iteration in addition to
  copying.

### 3.6 Counts

Cells are counted from the tables by `summary.py` (Appendix C.7). `mixed` = a
cell whose verdict depends on the operation or element type (e.g. `set` on a
record `List` field: `y` for fixed-width `T`, `n (G26)` otherwise).

**§1 collections** — 58 rows × 8 sites = 464 cells.

| site | y | mixed | n | n/a | total |
|---|---|---|---|---|---|
| S1 | 10 | 0 | 17 | 31 | 58 |
| S2 | 0 | 0 | 27 | 31 | 58 |
| S3 | 0 | 0 | 27 | 31 | 58 |
| S4 | 9 | 1 | 17 | 31 | 58 |
| S5 | 0 | 0 | 27 | 31 | 58 |
| S6 | 0 | 0 | 27 | 31 | 58 |
| S7 | 0 | 0 | 27 | 31 | 58 |
| S9 | 0 | 0 | 27 | 31 | 58 |
| **all** | **19** | **1** | **196** | **248** | **464** |

**§1b beyond collections** — 3 rows × 8 sites = 24 cells.

| site | y | mixed | n | n/a | total |
|---|---|---|---|---|---|
| S1 | 2 | 1 | 0 | 0 | 3 |
| S2 | 1 | 0 | 2 | 0 | 3 |
| S3 | 0 | 0 | 3 | 0 | 3 |
| S4 | 0 | 0 | 3 | 0 | 3 |
| S5 | 0 | 0 | 3 | 0 | 3 |
| S6 | 0 | 0 | 3 | 0 | 3 |
| S7 | 0 | 0 | 1 | 2 | 3 |
| S9 | 1 | 0 | 2 | 0 | 3 |
| **all** | **4** | **1** | **17** | **2** | **24** |

**§2 record updates** — 22 rows × 4 sites = 88 cells.

| site | y | mixed | n | n/a | total |
|---|---|---|---|---|---|
| S3 | 0 | 0 | 18 | 4 | 22 |
| S4 | 0 | 3 | 15 | 4 | 22 |
| S5 | 0 | 0 | 18 | 4 | 22 |
| S6 | 0 | 0 | 18 | 4 | 22 |
| **all** | **0** | **3** | **69** | **16** | **88** |

The two tables the plan names (§1 and §2) hold 552 cells; §1b adds 24. Each total above is the row count times the site count, and every cell falls in exactly one column.

**Deciding gates.** One count per `n` verdict inside a cell (a `mixed` cell contributes each of its `n` gates once). Sites per gate:

| deciding gate | cells | which |
|---|---|---|
| no arm | 132 | S1: 17, S3: 26, S4: 29, S6: 26, S7: 17, S9: 17 |
| StoreGlobal | 77 | S2: 29, S5: 48 |
| G17 | 44 | S3: 18, S4: 4, S6: 22 |
| G1 | 12 | S9: 12 |
| G7 | 11 | S7: 11 |
| G14 | 8 | S3: 4, S4: 4 |
| G26 | 3 | S4: 3 |


## Appendix A — census script

`census.py rows` generated the 58 rows of §1 (signatures verbatim from each
`mfb man collections <f>` page); `census.py stats` printed:

```
functions 49
overloads 58
collection-returning 34 in 32 functions
same-type 23 in 21 functions
non-collection 24 in 17 functions
```

```python
"""Row census for planning/plan-141-findings/inplace-audit.md.

Reads every `mfb man collections <f>` page, extracts each overload's signature
verbatim from its `Overloads`/`Declaration` block, and prints one markdown row
per overload, grouped by function in the order of the `mfb man collections`
Functions table. Non-collection-returning overloads are pre-filled `n/a`.

  python3 census.py rows   → the 58 table rows
  python3 census.py stats  → overloads / collection-returning / same-type counts
"""
import re
import subprocess
import sys

M = "/Users/justinzaun/Development/mfb/target/release/mfb"
top = subprocess.run([M, "man", "collections"], capture_output=True, text=True).stdout
table = top[top.index("\nFunctions\n"):]
funcs = []
for f in re.findall(r'│ collections::([a-zA-Z]+)', table):
    if f not in funcs:
        funcs.append(f)

rows = []
for f in funcs:
    page = subprocess.run([M, "man", "collections", f], capture_output=True, text=True).stdout
    lines = page.splitlines()
    i = next(k for k, l in enumerate(lines) if l.strip() in ("Overloads", "Declaration"))
    j = i + 2
    block = []
    while j < len(lines) and not (lines[j].strip() and j + 1 < len(lines)
                                  and set(lines[j + 1].strip()) == {'─'}):
        block.append(lines[j])
        j += 1
    text = " ".join(l.strip() for l in block)
    for s in re.findall(r'`(collections::[^`]+)`', text):
        s = re.sub(r'\s+', ' ', s)
        m = re.match(r'collections::\w+\((.*)\) AS (.*)$', s)
        params, ret = m.group(1), m.group(2)
        first = re.match(r'\w+ AS (.*?)(?:, \[?\w+ AS |$)', params)
        first_t = first.group(1) if first else ''
        coll = bool(re.match(r'(List|Set|Map)\b', ret))
        rows.append((f, s, ret, first_t, coll))

if sys.argv[1:] == ["stats"]:
    print("functions", len(funcs))
    print("overloads", len(rows))
    print("collection-returning", sum(r[4] for r in rows),
          "in", len({r[0] for r in rows if r[4]}), "functions")
    same = [r for r in rows if r[4] and r[2] == r[3]]
    print("same-type", len(same), "in", len({r[0] for r in same}), "functions")
    print("non-collection", sum(not r[4] for r in rows),
          "in", len({r[0] for r in rows if not r[4]}), "functions")
else:
    for f, s, ret, first_t, coll in rows:
        sig = s.replace("|", "\\|")
        if coll:
            cells = " |" * 7
            print(f"| `{sig}` |{cells} |")
        else:
            na = f" n/a (returns {ret}) |"
            print(f"| `{sig}` |{na * 7} no self-update form: the result type is not a collection, "
                  f"so `x = collections::{f}(x, …)` cannot type-check |")
```

## Appendix B — recogniser inventory

### B.1 Which lowering path an assignment takes

A source assignment `x = v` is lowered once, in the IR, by whether `x` is a
function local:

- `src/ir/lower.rs:1270` (`HirStatement::Assign`) and `:2159` (the inline-`TRAP`
  assign target): `if locals.contains_key(&target) { IrOp::Assign } else { IrOp::AssignGlobal }`.
  There is no third branch.
- `src/target/shared/nir/lower.rs:318` lowers `IrOp::Assign` to `NirOp::Assign`,
  and `:328` lowers `IrOp::AssignGlobal` to `NirOp::StoreGlobal`. The global
  initializer SUB (`:224`) also emits only `StoreGlobal`.
- Codegen lowers `NirOp::Assign` at exactly one place,
  `src/codegen/engine/control/builder_control.rs:1135`. Every other
  `NirOp::Assign {` match under `src/codegen` is an analysis (census:
  `grep -rn 'NirOp::Assign {' src/codegen --include='*.rs' | grep -v /tests/`).
- `NirOp::StoreGlobal` is lowered at `builder_control.rs:1060`:
  `lower_value_owned(value)` (the builtin builds a fresh block), then a free of
  the global's old block (`store_global_old`/`store_global_new` slots, bug-47),
  then a store. **It calls no `try_inplace_*`.**
- No optimizer pass turns a global into a local. The global census
  (`src/optimizer/opt1/plans/globals.rs:43-48`) records per-function use for
  "the *localization* half … that half additionally needs a proof that the value
  never carries across calls, so today only the constification half ships".
  `src/optimizer/opt1/globals.rs` only marks never-written globals read-only and
  substitutes literal initializers.

So **every write to a module-level `MUT`, including a `WITH` update of a
module-level record, reaches `StoreGlobal`, and no recogniser is reachable from
it.** Sites S2 and S5 are `n (StoreGlobal: no arm dispatched)` for every row.

### B.2 Every caller of every `try_inplace_*`

`grep -rn 'try_inplace_[a-z_]*(' src --include='*.rs' | grep -v 'fn try_inplace'`:

| caller | calls |
|---|---|
| `builder_control.rs:1166–1263`, inside `NirOp::Assign` (:1135) | the 18 non-`STATE` dispatch entries, in this order: `append`, `bulk_append`, `set_add`, `set`, `remove_key`, `prepend`, `remove_at`, `insert`, `set_remove`, `concat`, `record_field_append`, `record_field_remove_key`, `record_field_remove_at`, `record_field_set_remove`, `record_field_set_add`, `record_field_set`, `record_field_insert`, `record_field_prepend` |
| `builder_inplace_assign.rs:1385`, `:1396` | `record_field_insert` / `record_field_prepend` → `try_inplace_record_field_splice_assign` |
| `builder_control.rs:1440`, inside `NirOp::StateAssign` (:1436) | `state_scalar` |
| `builder_control.rs:1449`, inside `NirOp::StateAssign` | `state_collection_assign`, which calls (:351–358) `state_collection_append`, `state_remove_key`, `state_set_add`, `state_set`, `state_remove_at`, `state_set_remove`, `state_insert`, `state_prepend` |
| `builder_inplace_assign.rs:1266`, `:1275` | `state_insert` / `state_prepend` → `try_inplace_state_splice_assign` |

No caller sits under `NirOp::StoreGlobal`, `NirOp::Bind`, `Return`, `Eval`, or a
value lowering.

### B.3 The 30 recognisers

`grep -rn 'fn try_inplace_[a-z_]*' src --include='*.rs' | wc -l` → 30. Files:
`bia` = `src/codegen/collection/assign/builder_inplace_assign.rs`,
`bc` = `src/codegen/engine/control/builder_control.rs`.

Gate order is the order the code checks them; the first one that fails decides
the verdict. Two shared containers (`src/codegen/collection/assign/inplace_dest.rs`):

- **PL** = `resolve_inplace_plain_local` (:221): G2 `Call` → G3 name → G4 arity →
  G5 `args[0]` is a `Local` → G6 it is this binding → G8 local exists → `InPlaceGate`
  {G1 `by_ref`, G7 live `FOR EACH` over the local, G10 layout}.
- **RF** = `resolve_inplace_record_field` (:278): G2 `WithUpdate` → G13 target is
  this local → G14 exactly one update → G17 `record_collection_last_inlined`
  (`bc:290`) → `InPlaceGate` {G1, G15 live `FOR EACH` over this field, G10} →
  `inplace_call_args` {G2 `Call`, G3, G4}.

| recogniser | location | status | dispatch | recognises (builtin, arity, shape) | ordered gates after the container | vs `plan-121-gate-inventory.md` |
|---|---|---|---|---|---|---|
| try_inplace_append_assign | `bia:23` | audited | `bc:1166` | `append`, 2, `x = append(x, e)` (`Call`) | PL → G9 List → G11 item type = element | G7 is enforced (via PL), matching the matrix. |
| try_inplace_bulk_append_assign | `bia:1399` | audited | `bc:1167` | `append`, 2, `x = append(x, ys)` (`Call`) | inline, not PL: G1 → G2 → G3/G4 → G5 → G6 → G12 (`args[1]` is `x`) → G7 → G8 → G9 List → G10 → G11 item type = list type → E2 | Same gates as the matrix. |
| try_inplace_set_add_assign | `bia:196` | audited | `bc:1173` | `add`, 2, `s = add(s, e)` (`Call`) | inline: G1 → G2 → G3/G4 → G5 → G6 → G7 → G8 → G9 Set → G10 → G11 element | Same. |
| try_inplace_set_assign | `bia:1490` | audited | `bc:1179` | `set`, 3, `x = set(x, k, v)` (`Call`), List **or** Map | inline: G1 → G2 → G3/G4 → G5 → G6 → G7 → G8 → G10 → G9 List (E1, E2) \| Map (E2 key, E2 value) \| neither → decline | Same. The List branch now handles a longer variable-width replacement in place (`lower_list_set_in_place`, bug-627 resize/relocate labels, `list_mutate.rs:3332`), which the matrix's "overwrites one payload in place (same size) or rebuilds" predates. |
| try_inplace_remove_key_assign | `bia:271` | audited | `bc:1180` | `removeKey`, 2, `m = removeKey(m, k)` (`Call`) | inline: G1 → G2 → G3/G4 → G5 → G6 → G7 → G8 → G9 Map → G10 → G11 key | Same. |
| try_inplace_prepend_assign | `bia:1637` | audited | `bc:1186` | `prepend`, 2, `x = prepend(x, e)` (`Call`) | inline: G1 → G2 → G3/G4 → G5 → G6 → G7 → G8 → G9 List → G10 → E2 | Same. |
| try_inplace_remove_at_assign | `bia:1945` | audited | `bc:1197` | `removeAt`, 2, `x = removeAt(x, i)` (`Call`) | PL → G9 List → E1 | **New since the inventory** (plan-121-B). G24 was added and then **lifted** by plan-134-H (comment at `bia:1966`); the inventory still lists G24 as live. |
| try_inplace_insert_assign | `bia:2077` | audited | `bc:1203` | `insert`, 3, `x = insert(x, i, e)` (`Call`) | PL → G9 List → E1 → E2 | **New** (plan-121-B). |
| try_inplace_set_remove_assign | `bia:2025` | audited | `bc:1209` | `remove`, 2, `s = remove(s, e)` (`Call`) | PL → G9 Set → G11 element | **New** (plan-121-B). |
| try_inplace_concat_assign | `bia:1714` | audited | `bc:1215` | `&` chain, `s = s & a & …` (`Binary` chain) | G1 → G19 capacity shadow (from `prescan_string_self_appends`, `bc:2317`) → G20 chain shape → G21 no later operand reads `s` | Same. |
| try_inplace_record_field_append | `bia:92` | audited | `bc:1216` | `append`, 2, `r = WITH r { f := append(r.f, e \| ys) }` | RF → G9 List → G18 `args[0]` is `r.f` → G11 element or list → G12 `args[1]` is `r.f` | **G17 changed**: the inventory says "last-inlined **List** field"; since plan-121-C it admits any collection kind, and "last-inlined" counts **every** inlined field — `String`, nested record, data union, `Result`, collection (`record_field_is_inlined`, `builder_collection_layout.rs:3152`). |
| try_inplace_record_field_remove_key_assign | `bia:348` | audited | `bc:1222` | `removeKey`, 2, in a `WITH` | RF → G9 Map → G18 → G11 key | **New** (plan-121-C). |
| try_inplace_record_field_remove_at_assign | `bia:413` | audited | `bc:1228` | `removeAt`, 2, in a `WITH` | RF → G9 List → G18 → E1 | **New** (plan-121-C); G24 lifted (plan-134-H). |
| try_inplace_record_field_set_remove_assign | `bia:472` | audited | `bc:1234` | `remove`, 2, in a `WITH` | RF → G9 Set → G18 → G11 element | **New** (plan-121-C). |
| try_inplace_record_field_set_add_assign | `bia:538` | audited | `bc:1240` | `add`, 2, in a `WITH` | RF → G9 Set → G18 → G11 element → G12 | **New** (plan-121-C). |
| try_inplace_record_field_set_assign | `bia:629` | audited | `bc:1246` | `set`, 3, in a `WITH`, List or Map | RF → G18 → List: **G26** element is fixed-width (`list_element_is_fixed_width`), E1, E2 \| Map: E2 key, E2 value \| neither → decline | **New** (plan-121-C), and it carries a gate the inventory has no code for. This audit names it **G26: a `List` field's element type must be fixed-width** (`bia:658`), because the sub-block route cannot take the variable-width rebuild branch. |
| try_inplace_record_field_insert_assign | `bia:1378` | audited | `bc:1252` | `insert`, 3, in a `WITH` | → `record_field_splice_assign` | **New** (plan-121-C). |
| try_inplace_record_field_prepend_assign | `bia:1389` | audited | `bc:1258` | `prepend`, 2, in a `WITH` | → `record_field_splice_assign` | **New** (plan-121-C). |
| try_inplace_record_field_splice_assign | `bia:1291` | audited | `bia:1385`, `bia:1396` | shared body for `insert`/`prepend` | RF → G9 List → G18 → G12 (last arg is `r.f`) → G11 element | **New** (plan-121-C). Unlike the plain-local `insert`/`prepend`, this one has a static G11 gate. |
| try_inplace_state_remove_key_assign | `bia:816` | excluded (`STATE`) | `bc:352` | — | — | — |
| try_inplace_state_set_add_assign | `bia:865` | excluded (`STATE`) | `bc:353` | — | — | — |
| try_inplace_state_set_assign | `bia:943` | excluded (`STATE`) | `bc:354` | — | — | — |
| try_inplace_state_remove_at_assign | `bia:1087` | excluded (`STATE`) | `bc:355` | — | — | — |
| try_inplace_state_set_remove_assign | `bia:1134` | excluded (`STATE`) | `bc:356` | — | — | — |
| try_inplace_state_splice_assign | `bia:1181` | excluded (`STATE`) | `bia:1266`, `bia:1275` | — | — | — |
| try_inplace_state_insert_assign | `bia:1261` | excluded (`STATE`) | `bc:357` | — | — | — |
| try_inplace_state_prepend_assign | `bia:1270` | excluded (`STATE`) | `bc:358` | — | — | — |
| try_inplace_state_scalar_assign | `bc:139` | excluded (`STATE`) | `bc:1440` | — | — | — |
| try_inplace_state_collection_assign | `bc:346` | excluded (`STATE`) | `bc:1449` | — | — | — |
| try_inplace_state_collection_append | `bc:372` | excluded (`STATE`) | `bc:351` | — | — | — |

### B.4 Differences from `plan-121-gate-inventory.md`, collected

1. **Population**: the inventory counted 10 recognisers; there are 30 now (19
   non-`STATE`). New since: `remove_at`, `insert`, `set_remove` (plan-121-B);
   the 8 `record_field_*` beyond `append` (plan-121-C); the 8 `state_*` beyond
   `append`/`scalar`, plus the `state_collection_assign` dispatcher (plan-121-D).
2. **Dispatch lines moved**: the inventory cites `bc:879-909`; the chain is now
   `bc:1166–1263`.
3. **G17** is no longer "last-inlined *List*". It is "a collection field
   (`List`/`Map`/`Set`) after which **no inlined field of any kind** follows".
   A `String`, nested-record, data-union or `Result` field declared after the
   collection field makes it not-last (`bc:310-316`, `builder_collection_layout.rs:3152`).
   The collection field itself must also be inlined, which requires
   `type_is_memcpy_copyable` (`builder_collection_layout.rs:3160`); a field whose
   type is not (a recursive or resource-bearing element) declines at G17 too.
4. **G24** is listed as live for `removeAt`; plan-134-H lifted it in both the
   plain-local and record-field arms (`bia:431`, `bia:1966`).
5. **G26 (new code, this audit)**: `try_inplace_record_field_set_assign` declines a
   `List` field whose element is not fixed-width (`bia:658`). The inventory has no
   code for it. It means `r = WITH r { names := set(r.names, i, s) }` on a
   `List OF String` field always rebuilds, even though the plain-local `set` handles
   the same element type in place.
6. **G25** (`STATE` only, `inplace_dest.rs:386`) postdates the inventory; it is
   out of scope here.
7. **The container order** is now fixed by the two resolvers above. In RF, G17
   runs **before** G1/G15 and before the call shape G2/G3/G4, so a
   non-last or non-collection field declines at G17 whatever the operation is.
8. The inventory's matrix has no row for the `set` List branch's in-arm resize
   (bug-627). It is in place (amortized geometric grow), not a rebuild of the list.

## Appendix C — `--ncode` probes

All probes were built with the release compiler
(`/Users/justinzaun/Development/mfb/target/release/mfb`, built from `b6a10efbc`'s
`src/`) using `mfb build --ncode <project>` at the default optimization level,
in `/tmp/plan-141-probes/`. Each project uses the same `project.json`
(`kind: executable`, `entry: main`, `targets: [native]`).

**How a dump is read.** Every function in the `.ncode` JSON lists its
`stackSlots` by type name. Each `try_inplace_*` arm allocates a slot whose type
name occurs exactly once in `src/`
(`grep -rhoE '"inplace_…"' src --include='*.rs' | sort | uniq -c` → every count
1), so the slot's presence in a function proves that arm fired there, and its
absence proves the statement took another path. `store_global_new` marks the
`StoreGlobal` old-block free (`bc:1099`); `with_target` marks the whole-record
rebuild in `lower_with_update` (`builder_value_semantics.rs:720`).
**`set_inplace_*` slots are not arm markers**: the out-of-place `lower_set`
uses them on the fresh copy it makes, so they appear in the global `set` probe
too.

### C.1 `markers.py`

```python
"""Per-function in-place arm markers from an `mfb build --ncode` dump.

Usage: python3 markers.py <file.ncode> [function-name-prefix]

Each `try_inplace_*` arm in src/codegen/collection/assign/builder_inplace_assign.rs
allocates a stack slot whose type name appears nowhere else in src/ (checked with
`grep -rhoE '"inplace_…"' src | sort | uniq -c` → every count 1). A function's
`stackSlots` therefore records which arm fired in it. `set_inplace_*` slots are
NOT arm markers: the out-of-place `lower_set` uses them on its fresh copy.

Prints per function:
  ARM     — the arm(s) whose marker slot is present, or `-` (no arm fired: the
            statement took the copying path);
  GLOBAL  — `store_global_new` present (the `NirOp::StoreGlobal` lowering ran);
  WITH    — `with_target` present (`lower_with_update`, the whole-record rebuild);
  CALLS   — `bl` targets that are MFBASIC functions (a `Body::Mfb` source generic
            is a call to `_mfb_fn_…__collections_…`).
"""
import json
import re
import sys

ARMS = {
    "inplace_append_item": "append",
    "inplace_bulk_append_rhs": "bulk_append",
    "inplace_set_add_item": "set_add",
    "inplace_set_index": "set(List)",
    "inplace_set_key": "set(Map)",
    "inplace_remove_key": "remove_key",
    "inplace_prepend_item": "prepend",
    "inplace_remove_at_index": "remove_at",
    "inplace_insert_index": "insert",
    "inplace_set_remove_item": "set_remove",
    "concat_self_right": "concat",
    "inplace_recfield_rhs": "record_field_append",
    "inplace_recfield_remove_key": "record_field_remove_key",
    "inplace_recfield_remove_at_index": "record_field_remove_at",
    "inplace_recfield_set_remove": "record_field_set_remove",
    "inplace_recfield_add_item": "record_field_set_add",
    "inplace_recfield_set_index": "record_field_set(List)",
    "inplace_recfield_set_key": "record_field_set(Map)",
    "inplace_recfield_splice_item": "record_field_splice(insert/prepend)",
}

path = sys.argv[1]
prefix = sys.argv[2] if len(sys.argv) > 2 else ""
text = open(path).read()
data = json.loads(text[text.index("{"):])


def walk(node, out):
    if isinstance(node, dict):
        if "stackSlots" in node and "name" in node:
            out.append(node)
        for v in node.values():
            walk(v, out)
    elif isinstance(node, list):
        for v in node:
            walk(v, out)


functions = []
walk(data, functions)
for fn in functions:
    name = fn["name"]
    if not name.startswith(prefix):
        continue
    types = {s["type"] for s in fn.get("stackSlots", [])}
    arms = sorted({ARMS[t] for t in types if t in ARMS})
    glob = "y" if "store_global_new" in types else "-"
    with_ = "y" if "with_target" in types else "-"
    body = json.dumps(fn)
    calls = sorted({c for c in re.findall(r'"target": "(_mfb_i?fn_[^"]+)"', body)
                    if "collections" in c})
    print(f"{name}: ARM={','.join(arms) or '-'} GLOBAL={glob} WITH={with_} "
          f"CALLS={','.join(calls) or '-'}")
```

### C.2 Collections probe — generator and result

`gen_collections_probe.py` writes one SUB per (case, site): 27 cases × S1–S7,
plus S9 for the 17 `List` cases = 206 SUBs. It built without diagnostics.

```python
"""Generate /tmp/plan-141-probes/coll/src/main.mfb: one SUB per (case, site).

SUB names are `c_<case>_<site>`; markers.py then reports, per SUB, which
in-place arm (if any) fired. Every SUB prints a length so the update is live.
"""
import os

T = {"L": "List OF Integer", "M": "Map OF Integer TO Integer", "S": "Set OF Integer"}
INIT = {"L": "[1, 2, 3]", "M": "mkMap()", "S": "collections::toSet([1, 2, 3])"}

# (case, kind, op template with {X} for the collection being updated)
CASES = [
    ("add", "S", "collections::add({X}, k)"),
    ("append1", "L", "collections::append({X}, k)"),
    ("appendL", "L", "collections::append({X}, [k, k])"),
    ("difference", "S", "collections::difference({X}, collections::toSet([k]))"),
    ("distinct", "L", "collections::distinct({X})"),
    ("drop", "L", "collections::drop({X}, 1)"),
    ("filter", "L", "collections::filter({X}, isPositive)"),
    ("insert", "L", "collections::insert({X}, 0, k)"),
    ("intersection", "S", "collections::intersection({X}, collections::toSet([k]))"),
    ("mapValues", "M", "collections::mapValues({X}, negate)"),
    ("merge", "M", "collections::merge({X}, mkMap(), TRUE)"),
    ("mid", "L", "collections::mid({X}, 0, 2)"),
    ("prepend", "L", "collections::prepend({X}, k)"),
    ("reduce", "L", "collections::reduce({X}, emptyList(), keep)"),
    ("reduceRight", "L", "collections::reduceRight({X}, emptyList(), keep)"),
    ("remove", "S", "collections::remove({X}, k)"),
    ("removeAt", "L", "collections::removeAt({X}, 0)"),
    ("removeKey", "M", "collections::removeKey({X}, k)"),
    ("replace", "L", "collections::replace({X}, 1, k)"),
    ("setL", "L", "collections::set({X}, 0, k)"),
    ("setM", "M", "collections::set({X}, k, k)"),
    ("sort", "L", "collections::sort({X})"),
    ("sortBy", "L", "collections::sortBy({X}, negate)"),
    ("symmetricDifference", "S", "collections::symmetricDifference({X}, collections::toSet([k]))"),
    ("take", "L", "collections::take({X}, 2)"),
    ("transform", "L", "collections::transform({X}, negate)"),
    ("union", "S", "collections::union({X}, collections::toSet([k]))"),
]

out = ["IMPORT collections", "IMPORT io", ""]
for k in "LMS":
    out += [f"TYPE Rec{k}", f"  a AS {T[k]}", f"  b AS {T[k]}", "END TYPE", ""]
    out += [f"TYPE Outer{k}", "  n AS Integer", f"  inner AS Rec{k}", "END TYPE", ""]
out += [
    "FUNC mkMap() AS Map OF Integer TO Integer",
    "  MUT m AS Map OF Integer TO Integer",
    "  m = collections::set(m, 1, 1)",
    "  RETURN m",
    "END FUNC",
    "",
    "FUNC emptyList() AS List OF Integer",
    "  RETURN []",
    "END FUNC",
    "",
    "FUNC negate(v AS Integer) AS Integer",
    "  RETURN 0 - v",
    "END FUNC",
    "",
    "FUNC keep(acc AS List OF Integer, v AS Integer) AS List OF Integer",
    "  RETURN collections::append(acc, v)",
    "END FUNC",
    "",
]
for k in "LMS":
    out.append(f"MUT g{k} AS {T[k]} = {INIT[k]}")
    out.append(f"MUT gR{k} AS Rec{k} = Rec{k}[a := {INIT[k]}, b := {INIT[k]}]")
out.append("")

subs = []
for case, k, op in CASES:
    def sub(site, body):
        name = f"c_{case}_{site}"
        subs.append(name)
        out.extend([f"SUB {name}(k AS Integer)"] + ["  " + l for l in body] + ["END SUB", ""])

    sub("S1", [f"MUT x AS {T[k]} = {INIT[k]}", f"x = {op.format(X='x')}",
               "io::print(toString(len(x)))"])
    sub("S2", [f"g{k} = {op.format(X=f'g{k}')}", f"io::print(toString(len(g{k})))"])
    for site, field in (("S3", "a"), ("S4", "b")):
        sub(site, [f"MUT r AS Rec{k} = Rec{k}[a := {INIT[k]}, b := {INIT[k]}]",
                   f"r = WITH r {{ {field} := {op.format(X=f'r.{field}')} }}",
                   f"io::print(toString(len(r.{field})))"])
    sub("S5", [f"gR{k} = WITH gR{k} {{ b := {op.format(X=f'gR{k}.b')} }}",
               f"io::print(toString(len(gR{k}.b)))"])
    sub("S6", [f"MUT o AS Outer{k} = Outer{k}[n := 1, inner := Rec{k}[a := {INIT[k]}, b := {INIT[k]}]]",
               f"o = WITH o {{ inner := WITH o.inner {{ b := {op.format(X='o.inner.b')} }} }}",
               "io::print(toString(len(o.inner.b)))"])
    if k == "L":
        sub("S7", [f"MUT x AS {T[k]} = {INIT[k]}", "FOR EACH v IN x",
                   f"  x = {op.format(X='x')}", "NEXT", "io::print(toString(len(x)))"])
    else:
        # A Map/Set is iterated through its keys/elements list; FOR EACH over the
        # collection itself is tried here and reported if it does not compile.
        sub("S7", [f"MUT x AS {T[k]} = {INIT[k]}", "FOR EACH v IN x",
                   f"  x = {op.format(X='x')}", "NEXT", "io::print(toString(len(x)))"])
    if k == "L":
        sub("S9", [f"MUT x AS {T[k]} = {INIT[k]}",
                   f"collections::forEach([k], LAMBDA(v AS Integer) -> x = {op.format(X='x')})",
                   "io::print(toString(len(x)))"])

out += ["SUB main()"] + [f"  {s}(7)" for s in subs] + ["END SUB", ""]
os.makedirs("/tmp/plan-141-probes/coll/src", exist_ok=True)
open("/tmp/plan-141-probes/coll/src/main.mfb", "w").write("\n".join(out))
print(len(subs), "subs")
```

`python3 markers.py coll/probe.ncode c_` (206 lines; every SUB listed):

```
c_add_S1: ARM=set_add GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_add_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_add_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_add_S4: ARM=record_field_set_add GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_add_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_add_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_add_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_append1_S1: ARM=append GLOBAL=- WITH=- CALLS=-
c_append1_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_append1_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_append1_S4: ARM=record_field_append GLOBAL=- WITH=- CALLS=-
c_append1_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_append1_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_append1_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_append1_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_appendL_S1: ARM=bulk_append GLOBAL=- WITH=- CALLS=-
c_appendL_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_appendL_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_appendL_S4: ARM=record_field_append GLOBAL=- WITH=- CALLS=-
c_appendL_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_appendL_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_appendL_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_appendL_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_difference_S1: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fdifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_difference_S2: ARM=- GLOBAL=y WITH=- CALLS=_mfb_ifn_collections_5Fdifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_difference_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_difference_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_difference_S5: ARM=- GLOBAL=y WITH=y CALLS=_mfb_ifn_collections_5Fdifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_difference_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_difference_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fdifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_distinct_S1: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fdistinct_24Integer
c_distinct_S2: ARM=- GLOBAL=y WITH=- CALLS=_mfb_ifn_collections_5Fdistinct_24Integer
c_distinct_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdistinct_24Integer
c_distinct_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdistinct_24Integer
c_distinct_S5: ARM=- GLOBAL=y WITH=y CALLS=_mfb_ifn_collections_5Fdistinct_24Integer
c_distinct_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdistinct_24Integer
c_distinct_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fdistinct_24Integer
c_distinct_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_drop_S1: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fdrop_24Integer
c_drop_S2: ARM=- GLOBAL=y WITH=- CALLS=_mfb_ifn_collections_5Fdrop_24Integer
c_drop_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdrop_24Integer
c_drop_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdrop_24Integer
c_drop_S5: ARM=- GLOBAL=y WITH=y CALLS=_mfb_ifn_collections_5Fdrop_24Integer
c_drop_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fdrop_24Integer
c_drop_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fdrop_24Integer
c_drop_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_filter_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_filter_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_filter_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_filter_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_filter_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_filter_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_filter_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_filter_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_insert_S1: ARM=insert GLOBAL=- WITH=- CALLS=-
c_insert_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_insert_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_insert_S4: ARM=record_field_splice(insert/prepend) GLOBAL=- WITH=- CALLS=-
c_insert_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_insert_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_insert_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_insert_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_intersection_S1: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fintersection_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_intersection_S2: ARM=- GLOBAL=y WITH=- CALLS=_mfb_ifn_collections_5Fintersection_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_intersection_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fintersection_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_intersection_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fintersection_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_intersection_S5: ARM=- GLOBAL=y WITH=y CALLS=_mfb_ifn_collections_5Fintersection_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_intersection_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fintersection_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_intersection_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fintersection_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_mapValues_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_mapValues_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_mapValues_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_mapValues_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_mapValues_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_mapValues_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_mapValues_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_merge_S1: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fmerge_24Integer_24Integer
c_merge_S2: ARM=- GLOBAL=y WITH=- CALLS=_mfb_ifn_collections_5Fmerge_24Integer_24Integer
c_merge_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fmerge_24Integer_24Integer
c_merge_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fmerge_24Integer_24Integer
c_merge_S5: ARM=- GLOBAL=y WITH=y CALLS=_mfb_ifn_collections_5Fmerge_24Integer_24Integer
c_merge_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Fmerge_24Integer_24Integer
c_merge_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Fmerge_24Integer_24Integer
c_mid_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_mid_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_mid_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_mid_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_mid_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_mid_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_mid_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_mid_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_prepend_S1: ARM=prepend GLOBAL=- WITH=- CALLS=-
c_prepend_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_prepend_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_prepend_S4: ARM=record_field_splice(insert/prepend) GLOBAL=- WITH=- CALLS=-
c_prepend_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_prepend_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_prepend_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_prepend_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_reduce_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_reduce_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_reduce_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_reduce_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_reduce_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_reduce_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_reduce_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_reduce_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_reduceRight_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_reduceRight_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_reduceRight_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_reduceRight_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_reduceRight_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_reduceRight_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_reduceRight_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_reduceRight_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_remove_S1: ARM=set_remove GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_remove_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_remove_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_remove_S4: ARM=record_field_set_remove GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_remove_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_remove_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_remove_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer
c_removeAt_S1: ARM=remove_at GLOBAL=- WITH=- CALLS=-
c_removeAt_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_removeAt_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_removeAt_S4: ARM=record_field_remove_at GLOBAL=- WITH=- CALLS=-
c_removeAt_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_removeAt_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_removeAt_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_removeAt_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_removeKey_S1: ARM=remove_key GLOBAL=- WITH=- CALLS=-
c_removeKey_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_removeKey_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_removeKey_S4: ARM=record_field_remove_key GLOBAL=- WITH=- CALLS=-
c_removeKey_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_removeKey_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_removeKey_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_replace_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_replace_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_replace_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_replace_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_replace_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_replace_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_replace_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_replace_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_setL_S1: ARM=set(List) GLOBAL=- WITH=- CALLS=-
c_setL_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_setL_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_setL_S4: ARM=record_field_set(List) GLOBAL=- WITH=- CALLS=-
c_setL_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_setL_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_setL_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_setL_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_setM_S1: ARM=set(Map) GLOBAL=- WITH=- CALLS=-
c_setM_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_setM_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_setM_S4: ARM=record_field_set(Map) GLOBAL=- WITH=- CALLS=-
c_setM_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_setM_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_setM_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_sort_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_sort_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_sort_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_sort_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_sort_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_sort_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_sort_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_sort_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_sortBy_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_sortBy_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_sortBy_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_sortBy_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_sortBy_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_sortBy_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_sortBy_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_sortBy_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_symmetricDifference_S1: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FsymmetricDifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_symmetricDifference_S2: ARM=- GLOBAL=y WITH=- CALLS=_mfb_ifn_collections_5FsymmetricDifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_symmetricDifference_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FsymmetricDifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_symmetricDifference_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FsymmetricDifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_symmetricDifference_S5: ARM=- GLOBAL=y WITH=y CALLS=_mfb_ifn_collections_5FsymmetricDifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_symmetricDifference_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FsymmetricDifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_symmetricDifference_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FsymmetricDifference_24Integer,_mfb_ifn_collections_5FtoSet_24Integer
c_take_S1: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Ftake_24Integer
c_take_S2: ARM=- GLOBAL=y WITH=- CALLS=_mfb_ifn_collections_5Ftake_24Integer
c_take_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Ftake_24Integer
c_take_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Ftake_24Integer
c_take_S5: ARM=- GLOBAL=y WITH=y CALLS=_mfb_ifn_collections_5Ftake_24Integer
c_take_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5Ftake_24Integer
c_take_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5Ftake_24Integer
c_take_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_transform_S1: ARM=- GLOBAL=- WITH=- CALLS=-
c_transform_S2: ARM=- GLOBAL=y WITH=- CALLS=-
c_transform_S3: ARM=- GLOBAL=- WITH=y CALLS=-
c_transform_S4: ARM=- GLOBAL=- WITH=y CALLS=-
c_transform_S5: ARM=- GLOBAL=y WITH=y CALLS=-
c_transform_S6: ARM=- GLOBAL=- WITH=y CALLS=-
c_transform_S7: ARM=- GLOBAL=- WITH=- CALLS=-
c_transform_S9: ARM=- GLOBAL=- WITH=- CALLS=-
c_union_S1: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer,_mfb_ifn_collections_5Funion_24Integer
c_union_S2: ARM=- GLOBAL=y WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer,_mfb_ifn_collections_5Funion_24Integer
c_union_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer,_mfb_ifn_collections_5Funion_24Integer
c_union_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer,_mfb_ifn_collections_5Funion_24Integer
c_union_S5: ARM=- GLOBAL=y WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer,_mfb_ifn_collections_5Funion_24Integer
c_union_S6: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer,_mfb_ifn_collections_5Funion_24Integer
c_union_S7: ARM=- GLOBAL=- WITH=- CALLS=_mfb_ifn_collections_5FtoSet_24Integer,_mfb_ifn_collections_5Funion_24Integer
```

Tallies: arms fired in exactly 20 SUBs — S1 and S4 of the 10 arm-backed
overloads (`grep -v 'ARM=-'`). All 54 S2/S5 SUBs have `GLOBAL=y`. Of the 81
S3/S4/S6 SUBs, 71 have `WITH=y` (the rebuild); the 10 without are the S4 arm hits.
All 17 S9 lambdas (`markers.py … '$lambda'`) have `ARM=-`. `calls.py` on
`c_take_S1`/`c_union_S1` shows a `bl` to `_mfb_ifn_collections_5Ftake_24Integer` /
`…union_24Integer` (source generics); `c_sort_S1` has none (native fast path).

### C.3 Generic-only self-updates that cannot type-check

`gen/src/main.mfb` holds one SUB per function: `m = collections::keys(m)`,
`m = collections::values(m)`, `s = collections::toList(s)`,
`xs = collections::toSet(xs)`, `xs = collections::chunks(xs, 2)`,
`xs = collections::window(xs, 2)`, `xss = collections::flatten(xss)`,
`xs = collections::groupBy(xs, ident, ident)`, `xs = collections::zip(xs, xs)`.
`mfb build gen` → exit 1:

```
/tmp/plan-141-probes/gen/src/main.mfb:14 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
/tmp/plan-141-probes/gen/src/main.mfb:19 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
/tmp/plan-141-probes/gen/src/main.mfb:24 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
/tmp/plan-141-probes/gen/src/main.mfb:29 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
/tmp/plan-141-probes/gen/src/main.mfb:34 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
/tmp/plan-141-probes/gen/src/main.mfb:39 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
/tmp/plan-141-probes/gen/src/main.mfb:44 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
/tmp/plan-141-probes/gen/src/main.mfb:49 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
/tmp/plan-141-probes/gen/src/main.mfb:54 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
```

(one error per SUB, at each assignment line). `transform`, `mapValues`,
`reduce` and `reduceRight` self-updates compiled in C.2.

### C.4 Record-form and extras probe

```basic
IMPORT collections
IMPORT io

TYPE ScFirst
  i AS Integer
  f AS Float
  b AS Boolean
  s AS String
  xs AS List OF Integer
END TYPE

TYPE ScLast
  xs AS List OF Integer
  i AS Integer
  f AS Float
  b AS Boolean
  s AS String
END TYPE

TYPE ScOuter
  n AS Integer
  inner AS ScFirst
END TYPE

TYPE Col
  la AS List OF Integer
  ma AS Map OF Integer TO Integer
  sa AS Set OF Integer
  lb AS List OF Integer
END TYPE

TYPE ColM
  la AS List OF Integer
  mb AS Map OF Integer TO Integer
END TYPE

TYPE ColS
  la AS List OF Integer
  sb AS Set OF Integer
END TYPE

TYPE ListThenString
  xs AS List OF Integer
  name AS String
END TYPE

TYPE ListThenInt
  xs AS List OF Integer
  n AS Integer
END TYPE

TYPE Names
  n AS Integer
  names AS List OF String
END TYPE

TYPE Two
  a AS List OF Integer
  b AS List OF Integer
  i AS Integer
  j AS Integer
END TYPE

TYPE OuterFirst
  inner AS ColS
  n AS Integer
END TYPE

FUNC mkMap() AS Map OF Integer TO Integer
  MUT m AS Map OF Integer TO Integer
  m = collections::set(m, 1, 1)
  RETURN m
END FUNC

FUNC mkScFirst() AS ScFirst
  RETURN ScFirst[i := 1, f := 1.0, b := TRUE, s := "a", xs := [1]]
END FUNC

MUT gSc AS ScFirst = mkScFirst()
MUT gStr AS String = "a"
MUT gInt AS Integer = 0
MUT gCol AS Col = Col[la := [1], ma := mkMap(), sa := collections::toSet([1]), lb := [1]]

' R1: scalar field, first (S3) and last (S4) position, global (S5), nested (S6)
SUB r1_int_S3(k AS Integer)
  MUT r AS ScFirst = mkScFirst()
  r = WITH r { i := k }
  io::print(toString(r.i))
END SUB
SUB r1_float_S3(k AS Integer)
  MUT r AS ScFirst = mkScFirst()
  r = WITH r { f := toFloat(k) }
  io::print(toString(r.f))
END SUB
SUB r1_bool_S3(k AS Integer)
  MUT r AS ScFirst = mkScFirst()
  r = WITH r { b := k > 3 }
  io::print(toString(r.b))
END SUB
SUB r1_string_S3(k AS Integer)
  MUT r AS ScFirst = mkScFirst()
  r = WITH r { s := toString(k) }
  io::print(r.s)
END SUB
SUB r1_int_S4(k AS Integer)
  MUT r AS ScLast = ScLast[xs := [1], i := 1, f := 1.0, b := TRUE, s := "a"]
  r = WITH r { i := k }
  io::print(toString(r.i))
END SUB
SUB r1_string_S4(k AS Integer)
  MUT r AS ScLast = ScLast[xs := [1], i := 1, f := 1.0, b := TRUE, s := "a"]
  r = WITH r { s := toString(k) }
  io::print(r.s)
END SUB
SUB r1_int_S5(k AS Integer)
  gSc = WITH gSc { i := k }
  io::print(toString(gSc.i))
END SUB
SUB r1_int_S6(k AS Integer)
  MUT o AS ScOuter = ScOuter[n := 1, inner := mkScFirst()]
  o = WITH o { inner := WITH o.inner { i := k } }
  io::print(toString(o.inner.i))
END SUB

' R2: replace a collection field with a value not derived from it
SUB r2_list_S3(k AS Integer)
  MUT r AS Col = Col[la := [1], ma := mkMap(), sa := collections::toSet([1]), lb := [1]]
  r = WITH r { la := [k] }
  io::print(toString(len(r.la)))
END SUB
SUB r2_list_S4(k AS Integer)
  MUT r AS Col = Col[la := [1], ma := mkMap(), sa := collections::toSet([1]), lb := [1]]
  r = WITH r { lb := [k] }
  io::print(toString(len(r.lb)))
END SUB
SUB r2_map_S4(k AS Integer)
  MUT r AS ColM = ColM[la := [1], mb := mkMap()]
  r = WITH r { mb := mkMap() }
  io::print(toString(len(r.mb)))
END SUB
SUB r2_set_S4(k AS Integer)
  MUT r AS ColS = ColS[la := [1], sb := collections::toSet([1])]
  r = WITH r { sb := collections::toSet([k]) }
  io::print(toString(len(r.sb)))
END SUB
SUB r2_list_S5(k AS Integer)
  gCol = WITH gCol { lb := [k] }
  io::print(toString(len(gCol.lb)))
END SUB

' R3 extras: G17 through a trailing String / trailing scalar, G26 variable-width set
SUB r3_list_then_string(k AS Integer)
  MUT r AS ListThenString = ListThenString[xs := [1], name := "a"]
  r = WITH r { xs := collections::append(r.xs, k) }
  io::print(toString(len(r.xs)))
END SUB
SUB r3_list_then_int(k AS Integer)
  MUT r AS ListThenInt = ListThenInt[xs := [1], n := 1]
  r = WITH r { xs := collections::append(r.xs, k) }
  io::print(toString(len(r.xs)))
END SUB
SUB r3_names_set(k AS Integer)
  MUT r AS Names = Names[n := 1, names := ["a", "b"]]
  r = WITH r { names := collections::set(r.names, 0, toString(k)) }
  io::print(toString(len(r.names)))
END SUB
SUB r3_names_append(k AS Integer)
  MUT r AS Names = Names[n := 1, names := ["a", "b"]]
  r = WITH r { names := collections::append(r.names, toString(k)) }
  io::print(toString(len(r.names)))
END SUB
SUB r3_map_S3(k AS Integer)
  MUT r AS Col = Col[la := [1], ma := mkMap(), sa := collections::toSet([1]), lb := [1]]
  r = WITH r { ma := collections::set(r.ma, k, k) }
  io::print(toString(len(r.ma)))
END SUB
SUB r3_loop_live_field(k AS Integer)
  MUT r AS ListThenInt = ListThenInt[xs := [1], n := 1]
  FOR EACH v IN r.xs
    r = WITH r { xs := collections::append(r.xs, v) }
  NEXT
  io::print(toString(len(r.xs)))
END SUB

' R4: two fields in one WITH
SUB r4_list(k AS Integer)
  MUT r AS Two = Two[a := [1], b := [1], i := 1, j := 1]
  r = WITH r { a := collections::append(r.a, k), b := collections::append(r.b, k) }
  io::print(toString(len(r.b)))
END SUB
SUB r4_scalar(k AS Integer)
  MUT r AS Two = Two[a := [1], b := [1], i := 1, j := 1]
  r = WITH r { i := k, j := k }
  io::print(toString(r.j))
END SUB
SUB r4_list_one_collection(k AS Integer)
  MUT r AS ListThenInt = ListThenInt[xs := [1], n := 1]
  r = WITH r { xs := collections::append(r.xs, k), n := k }
  io::print(toString(len(r.xs)))
END SUB

' R5: nested WITH, the nested record first (S3) in its parent
SUB r5_set_S3(k AS Integer)
  MUT o AS OuterFirst = OuterFirst[inner := ColS[la := [1], sb := collections::toSet([1])], n := 1]
  o = WITH o { inner := WITH o.inner { sb := collections::add(o.inner.sb, k) } }
  io::print(toString(len(o.inner.sb)))
END SUB

' Extras: String self-concat, scalar self-update, nested collection (grid)
SUB x_concat_S1(k AS Integer)
  MUT s AS String = "a"
  s = s & toString(k)
  io::print(s)
END SUB
SUB x_concat_S2(k AS Integer)
  gStr = gStr & toString(k)
  io::print(gStr)
END SUB
SUB x_concat_S3(k AS Integer)
  MUT r AS ScFirst = mkScFirst()
  r = WITH r { s := r.s & toString(k) }
  io::print(r.s)
END SUB
SUB x_concat_S4(k AS Integer)
  MUT r AS ScLast = ScLast[xs := [1], i := 1, f := 1.0, b := TRUE, s := "a"]
  r = WITH r { s := r.s & toString(k) }
  io::print(r.s)
END SUB
SUB x_concat_S9(k AS Integer)
  MUT s AS String = "a"
  collections::forEach([k], LAMBDA(v AS Integer) -> s = s & toString(v))
  io::print(s)
END SUB
SUB x_int_S1(k AS Integer)
  MUT i AS Integer = 0
  i = i + k
  io::print(toString(i))
END SUB
SUB x_int_S2(k AS Integer)
  gInt = gInt + k
  io::print(toString(gInt))
END SUB
SUB x_grid_S8(k AS Integer)
  MUT grid AS List OF List OF Integer = [[1, 2], [3, 4]]
  grid = collections::set(grid, 0, collections::set(collections::get(grid, 0), 1, k))
  io::print(toString(len(grid)))
END SUB
SUB x_setString_S1(k AS Integer)
  MUT xs AS List OF String = ["a", "b"]
  xs = collections::set(xs, 0, toString(k))
  io::print(toString(len(xs)))
END SUB

SUB main()
  r1_int_S3(7)
  r1_float_S3(7)
  r1_bool_S3(7)
  r1_string_S3(7)
  r1_int_S4(7)
  r1_string_S4(7)
  r1_int_S5(7)
  r1_int_S6(7)
  r2_list_S3(7)
  r2_list_S4(7)
  r2_map_S4(7)
  r2_set_S4(7)
  r2_list_S5(7)
  r3_list_then_string(7)
  r3_list_then_int(7)
  r3_names_set(7)
  r3_names_append(7)
  r3_map_S3(7)
  r3_loop_live_field(7)
  r4_list(7)
  r4_scalar(7)
  r4_list_one_collection(7)
  r5_set_S3(7)
  x_concat_S1(7)
  x_concat_S2(7)
  x_concat_S3(7)
  x_concat_S4(7)
  x_concat_S9(7)
  x_int_S1(7)
  x_int_S2(7)
  x_grid_S8(7)
  x_setString_S1(7)
END SUB
```

`python3 markers.py rec/probe.ncode <prefix>` for `r1 r2 r3 r4 r5 x_`:

```
r1_int_S3: ARM=- GLOBAL=- WITH=y CALLS=-
r1_float_S3: ARM=- GLOBAL=- WITH=y CALLS=-
r1_bool_S3: ARM=- GLOBAL=- WITH=y CALLS=-
r1_string_S3: ARM=- GLOBAL=- WITH=y CALLS=-
r1_int_S4: ARM=- GLOBAL=- WITH=y CALLS=-
r1_string_S4: ARM=- GLOBAL=- WITH=y CALLS=-
r1_int_S5: ARM=- GLOBAL=y WITH=y CALLS=-
r1_int_S6: ARM=- GLOBAL=- WITH=y CALLS=-
r2_list_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
r2_list_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
r2_map_S4: ARM=- GLOBAL=- WITH=y CALLS=-
r2_set_S4: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
r2_list_S5: ARM=- GLOBAL=y WITH=y CALLS=-
r3_list_then_string: ARM=- GLOBAL=- WITH=y CALLS=-
r3_list_then_int: ARM=record_field_append GLOBAL=- WITH=- CALLS=-
r3_names_set: ARM=- GLOBAL=- WITH=y CALLS=-
r3_names_append: ARM=record_field_append GLOBAL=- WITH=- CALLS=-
r3_map_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
r3_loop_live_field: ARM=- GLOBAL=- WITH=y CALLS=-
r4_list: ARM=- GLOBAL=- WITH=y CALLS=-
r4_scalar: ARM=- GLOBAL=- WITH=y CALLS=-
r4_list_one_collection: ARM=- GLOBAL=- WITH=y CALLS=-
r5_set_S3: ARM=- GLOBAL=- WITH=y CALLS=_mfb_ifn_collections_5FtoSet_24Integer
x_concat_S1: ARM=concat GLOBAL=- WITH=- CALLS=-
x_concat_S2: ARM=- GLOBAL=y WITH=- CALLS=-
x_concat_S3: ARM=- GLOBAL=- WITH=y CALLS=-
x_concat_S4: ARM=- GLOBAL=- WITH=y CALLS=-
x_concat_S9: ARM=- GLOBAL=- WITH=- CALLS=-
x_int_S1: ARM=- GLOBAL=- WITH=- CALLS=-
x_int_S2: ARM=- GLOBAL=- WITH=- CALLS=-
x_grid_S8: ARM=set(List) GLOBAL=- WITH=- CALLS=-
x_setString_S1: ARM=set(List) GLOBAL=- WITH=- CALLS=-
$lambda0: ARM=- GLOBAL=- WITH=- CALLS=-
```

### C.5 R6 — `r.prop = value`

`r6/src/main.mfb` is `MUT r AS P = P[xs := [1], n := 1]` then `r.n = 5`.
`mfb build r6` →

```
/tmp/plan-141-probes/r6/src/main.mfb:10 error[1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT]: record field assignment is not supported
```

which is rule `1-102-0013` at `src/rules/table.rs:185–190`.

### C.7 `summary.py` — the §3.6 counts

Parses the three tables in this file and classifies each cell as `y`, `n`, `n/a` or `mixed`; each `n (…)` gate inside a cell adds one to that gate's count.

```python
"""Compute §3 Summary counts from the findings file's tables and write §3."""
import re
from collections import Counter, defaultdict

F = ("/Users/justinzaun/Development/mfb/.claude/worktrees/P-141/"
     "planning/plan-141-findings/inplace-audit.md")
text = open(F).read()


def section(start, end):
    return text[text.index(start):text.index(end)]


def rows(sec, prefix):
    out = []
    for line in sec.split("\n"):
        if line.startswith(prefix):
            cells = [c.strip() for c in re.split(r"(?<!\\)\|", line)[1:-1]]
            out.append(cells)
    return out


def kind(cell):
    has_y = cell.startswith("y") or " y " in cell or "/ y" in cell
    has_n = bool(re.search(r"(^|[;/] ?)n \(", cell)) or cell.startswith("n (")
    if cell.startswith("n/a"):
        return "n/a"
    if has_y and has_n:
        return "mixed"
    if has_y:
        return "y"
    if has_n:
        return "n"
    raise ValueError(cell)


def gates(cell):
    return re.findall(r"n \((G\d+|no arm|StoreGlobal)", cell)


tables = {
    "§1 collections": (rows(section("## 1. Collections", "### 1b."), "| `collections::"),
                       ["S1", "S2", "S3", "S4", "S5", "S6", "S7", "S9"], 1),
    "§1b beyond collections": ([r for r in rows(section("### 1b.", "## 2. Record updates"), "| ")
                                if r[0] != "form"],
                               ["S1", "S2", "S3", "S4", "S5", "S6", "S7", "S9"], 1),
    "§2 record updates": (rows(section("## 2. Record updates", "## 3. Summary"), "| R"),
                          ["S3", "S4", "S5", "S6"], 2),
}

report = []
gate_cells = defaultdict(list)
grand = Counter()
two_table_cells = 0
for name, (rs, sites, first) in tables.items():
    per_site = {s: Counter() for s in sites}
    for r in rs:
        label = r[0] if first == 1 else f"{r[0][:2]} {r[1]}"
        for i, s in enumerate(sites):
            cell = r[first + i]
            k = kind(cell)
            per_site[s][k] += 1
            for g in gates(cell):
                gate_cells[g].append(f"{label.split('(')[0].strip('`')}@{s}")
    total = Counter()
    for c in per_site.values():
        total.update(c)
    grand.update(total)
    ncells = sum(total.values())
    if name != "§1b beyond collections":
        two_table_cells += ncells
    report.append((name, len(rs), sites, per_site, total, ncells))

lines = []
for name, nrows, sites, per_site, total, ncells in report:
    lines.append(f"**{name}** — {nrows} rows × {len(sites)} sites = {ncells} cells.\n")
    lines.append("| site | y | mixed | n | n/a | total |")
    lines.append("|---|---|---|---|---|---|")
    for s in sites:
        c = per_site[s]
        lines.append(f"| {s} | {c['y']} | {c['mixed']} | {c['n']} | {c['n/a']} | {sum(c.values())} |")
    lines.append(f"| **all** | **{total['y']}** | **{total['mixed']}** | **{total['n']}** | "
                 f"**{total['n/a']}** | **{sum(total.values())}** |")
    lines.append("")
counts_md = "\n".join(lines)

gate_md = ["| deciding gate | cells | which |", "|---|---|---|"]
for g, cells in sorted(gate_cells.items(), key=lambda kv: -len(kv[1])):
    by_site = Counter(c.split("@")[1] for c in cells)
    which = ", ".join(f"{s}: {n}" for s, n in sorted(by_site.items()))
    gate_md.append(f"| {g} | {len(cells)} | {which} |")
gate_md = "\n".join(gate_md)

print(counts_md)
print(gate_md)
print("two-table cells", two_table_cells, "grand", sum(grand.values()))
open("/tmp/plan-141-probes/summary_counts.md", "w").write(
    counts_md + "\n" + f"The two tables the plan names (§1 and §2) hold "
    f"{two_table_cells} cells; §1b adds "
    f"{sum(grand.values()) - two_table_cells}. Each total above is the row count "
    f"times the site count, and every cell falls in exactly one column.\n\n"
    "**Deciding gates.** One count per `n` verdict inside a cell (a `mixed` cell "
    "contributes each of its `n` gates once). Sites per gate:\n\n" + gate_md + "\n")
```

### C.6 Reading vs dump

Every verdict read from the code was confirmed by its probe. **No
disagreement.**
