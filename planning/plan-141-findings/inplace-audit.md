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

`STATE` sites are excluded (plan §1 non-goals).

Cell values: `y` = in place; `n (Gxx)` = the named gate declines and the
statement takes the copying path; `n (no arm)` = no recogniser exists for this
function, so the statement always takes the copying path; `n/a` = the form
cannot be written (reason given).

## 1. Collections

| function definition | S1 | S2 | S3 | S4 | S5 | S6 | S7 | evidence |
|---|---|---|---|---|---|---|---|---|
| `collections::add(value AS Set OF T, item AS T) AS Set OF T` | | | | | | | | |
| `collections::all(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::all(x, …)` cannot type-check |
| `collections::any(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::any(x, …)` cannot type-check |
| `collections::append(value AS List OF T, item AS T) AS List OF T` | | | | | | | | |
| `collections::append(value AS List OF T, item AS List OF T) AS List OF T` | | | | | | | | |
| `collections::chunks(value AS List OF T, chunkSize AS Integer) AS List OF List OF T` | | | | | | | | |
| `collections::contains(value AS List OF T, item AS T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::contains(x, …)` cannot type-check |
| `collections::contains(value AS Set OF T, item AS T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::contains(x, …)` cannot type-check |
| `collections::difference(a AS Set OF T, b AS Set OF T) AS Set OF T` | | | | | | | | |
| `collections::distinct(value AS List OF T) AS List OF T` | | | | | | | | |
| `collections::drop(value AS List OF T, count AS Integer) AS List OF T` | | | | | | | | |
| `collections::filter(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS List OF T` | | | | | | | | |
| `collections::find(value AS List OF T, item AS T, [start AS Integer]) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::find(x, …)` cannot type-check |
| `collections::find(value AS List OF T, item AS List OF T, [start AS Integer]) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::find(x, …)` cannot type-check |
| `collections::findIndex(value AS List OF T, predicate AS FUNC(T) AS Boolean, [start AS Integer]) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::findIndex(x, …)` cannot type-check |
| `collections::findLastIndex(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::findLastIndex(x, …)` cannot type-check |
| `collections::findLastIndex(value AS List OF T, predicate AS FUNC(T) AS Boolean, start AS Integer) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::findLastIndex(x, …)` cannot type-check |
| `collections::flatten(value AS List OF List OF T) AS List OF T` | | | | | | | | |
| `collections::forEach(value AS List OF T, action AS FUNC(T) AS Nothing) AS Nothing` | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | n/a (returns Nothing) | no self-update form: the result type is not a collection, so `x = collections::forEach(x, …)` cannot type-check |
| `collections::get(value AS List OF T, index AS Integer) AS T` | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | no self-update form: the result type is not a collection, so `x = collections::get(x, …)` cannot type-check |
| `collections::get(value AS Map OF K TO V, index AS K) AS V` | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | no self-update form: the result type is not a collection, so `x = collections::get(x, …)` cannot type-check |
| `collections::getOr(value AS List OF T, index AS Integer, default AS T) AS T` | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | n/a (returns T) | no self-update form: the result type is not a collection, so `x = collections::getOr(x, …)` cannot type-check |
| `collections::getOr(value AS Map OF K TO V, index AS K, default AS V) AS V` | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | n/a (returns V) | no self-update form: the result type is not a collection, so `x = collections::getOr(x, …)` cannot type-check |
| `collections::groupBy(value AS List OF T, keyFn AS FUNC(T) AS K, valFn AS FUNC(T) AS V) AS Map OF K TO List OF V` | | | | | | | | |
| `collections::hasKey(value AS Map OF K TO V, key AS K) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::hasKey(x, …)` cannot type-check |
| `collections::insert(value AS List OF T, index AS Integer, item AS T) AS List OF T` | | | | | | | | |
| `collections::intersection(a AS Set OF T, b AS Set OF T) AS Set OF T` | | | | | | | | |
| `collections::isDisjoint(a AS Set OF T, b AS Set OF T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::isDisjoint(x, …)` cannot type-check |
| `collections::isSubset(a AS Set OF T, b AS Set OF T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::isSubset(x, …)` cannot type-check |
| `collections::isSuperset(a AS Set OF T, b AS Set OF T) AS Boolean` | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | n/a (returns Boolean) | no self-update form: the result type is not a collection, so `x = collections::isSuperset(x, …)` cannot type-check |
| `collections::keys(value AS Map OF K TO V) AS List OF K` | | | | | | | | |
| `collections::mapValues(value AS Map OF K TO V, f AS FUNC(V) AS U) AS Map OF K TO U` | | | | | | | | |
| `collections::merge(a AS Map OF K TO V, b AS Map OF K TO V, preferB AS Boolean) AS Map OF K TO V` | | | | | | | | |
| `collections::mid(value AS List OF T, start AS Integer, count AS Integer) AS List OF T` | | | | | | | | |
| `collections::partition(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Partition OF T` | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | n/a (returns Partition OF T) | no self-update form: the result type is not a collection, so `x = collections::partition(x, …)` cannot type-check |
| `collections::prepend(value AS List OF T, item AS T) AS List OF T` | | | | | | | | |
| `collections::reduce(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` | | | | | | | | |
| `collections::reduceRight(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` | | | | | | | | |
| `collections::remove(value AS Set OF T, item AS T) AS Set OF T` | | | | | | | | |
| `collections::removeAt(value AS List OF T, index AS Integer) AS List OF T` | | | | | | | | |
| `collections::removeKey(value AS Map OF K TO V, key AS K) AS Map OF K TO V` | | | | | | | | |
| `collections::replace(value AS List OF T, old AS T, new AS T) AS List OF T` | | | | | | | | |
| `collections::set(value AS List OF T, index AS Integer, item AS T) AS List OF T` | | | | | | | | |
| `collections::set(value AS Map OF K TO V, index AS K, item AS V) AS Map OF K TO V` | | | | | | | | |
| `collections::sort(value AS List OF T) AS List OF T` | | | | | | | | |
| `collections::sortBy(value AS List OF T, keyFn AS FUNC(T) AS U) AS List OF T` | | | | | | | | |
| `collections::sum(value AS List OF Integer) AS Integer` | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | n/a (returns Integer) | no self-update form: the result type is not a collection, so `x = collections::sum(x, …)` cannot type-check |
| `collections::sum(value AS List OF Float) AS Float` | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | n/a (returns Float) | no self-update form: the result type is not a collection, so `x = collections::sum(x, …)` cannot type-check |
| `collections::sum(value AS List OF Fixed) AS Fixed` | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | n/a (returns Fixed) | no self-update form: the result type is not a collection, so `x = collections::sum(x, …)` cannot type-check |
| `collections::symmetricDifference(a AS Set OF T, b AS Set OF T) AS Set OF T` | | | | | | | | |
| `collections::take(value AS List OF T, count AS Integer) AS List OF T` | | | | | | | | |
| `collections::toList(value AS Set OF T) AS List OF T` | | | | | | | | |
| `collections::toSet(value AS List OF T) AS Set OF T` | | | | | | | | |
| `collections::transform(value AS List OF T, f AS FUNC(T) AS U) AS List OF U` | | | | | | | | |
| `collections::union(a AS Set OF T, b AS Set OF T) AS Set OF T` | | | | | | | | |
| `collections::values(value AS Map OF K TO V) AS List OF V` | | | | | | | | |
| `collections::window(value AS List OF T, size AS Integer, [stride AS Integer]) AS List OF List OF T` | | | | | | | | |
| `collections::zip(a AS List OF A, b AS List OF B) AS List OF Pair OF A, B` | | | | | | | | |

## 2. Record updates

| form | field type | S3 | S4 | S5 | S6 | evidence |
|---|---|---|---|---|---|---|
| R1 `r = WITH r { f := <new scalar> }` | Integer | | | | | |
| R1 `r = WITH r { f := <new scalar> }` | Float | | | | | |
| R1 `r = WITH r { f := <new scalar> }` | Boolean | | | | | |
| R1 `r = WITH r { f := <new scalar> }` | String | | | | | |
| R2 `r = WITH r { f := <new value not derived from r.f> }` | List | | | | | |
| R2 `r = WITH r { f := <new value not derived from r.f> }` | Map | | | | | |
| R2 `r = WITH r { f := <new value not derived from r.f> }` | Set | | | | | |
| R3 `r = WITH r { f := collections::<op>(r.f, …) }` | List | | | | | |
| R3 `r = WITH r { f := collections::<op>(r.f, …) }` | Map | | | | | |
| R3 `r = WITH r { f := collections::<op>(r.f, …) }` | Set | | | | | |
| R4 `r = WITH r { f := …, g := … }` | List | | | | | |
| R4 `r = WITH r { f := …, g := … }` | Map | | | | | |
| R4 `r = WITH r { f := …, g := … }` | Set | | | | | |
| R4 `r = WITH r { f := …, g := … }` | scalar | | | | | |
| R5 `r = WITH r { inner := WITH r.inner { f := … } }` | List | | | | | |
| R5 `r = WITH r { inner := WITH r.inner { f := … } }` | Map | | | | | |
| R5 `r = WITH r { inner := WITH r.inner { f := … } }` | Set | | | | | |
| R5 `r = WITH r { inner := WITH r.inner { f := … } }` | scalar | | | | | |
| R6 `r.prop = value` | List | | | | | |
| R6 `r.prop = value` | Map | | | | | |
| R6 `r.prop = value` | Set | | | | | |
| R6 `r.prop = value` | scalar | | | | | |

## 3. Summary

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
