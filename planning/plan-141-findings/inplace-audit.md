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

## Appendix C — `--ncode` probes
