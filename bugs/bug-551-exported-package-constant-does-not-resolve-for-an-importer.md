# bug-551: an `EXPORT LET` package constant is visible to an importer but has no type — `pkg::Name` dies with `TYPE_UNKNOWN_VALUE`

Last updated: 2026-09-06
Effort: medium (front end + IR merge; the wire format already carries what is needed)
Severity: MEDIUM (a specified language feature is unusable; every package must work around it)
Class: Unimplemented spec surface

Status: FIXED (branch `fix-pkg-symbol-resolution`). Point 3 — the initialization
order this was filed rather than fixed over — was measured first and holds; see
"Point 3, measured" below. Points 1, 2 and 4 are all implemented.
Regression Test: `tests/rt_imported_package_global.rs` — 5 positive (read of an
`EXPORT LET`, the initialization-order measurement, the `EXPORT MUT` write/read
round trip through the package's OWN accessor, an aliased `IMPORT … AS`, and `=`
still being equality in expression position) and 5 negative (a write to an
`EXPORT LET`, a wrong-typed write, a wrong-typed read, a `PRIVATE` global, an
unexported name), all confirmed RED against `8a7735c85`'s binary via
`MFB_TEST_EXE`.

## The finding

`mfb spec language modules-and-packages` (§13) says an exported top-level `LET`
is importer-visible, and that the qualified-name rule covers constants:

> Top-level LET, MUT, FUNC, SUB, TYPE, UNION, ENUM, and RESOURCE may use
> PRIVATE, PUBLIC, or EXPORT.

> Where a name is defined decides whether it needs a prefix. … This holds for
> every kind of name alike: **variables and constants**, functions, records,
> unions, union variants, enums, enum members, and resource types.

It does not work. The name resolves as a member of the package's export surface,
but it carries no type, so every use dies at the *use site* with
`TYPE_UNKNOWN_VALUE` (or as an `(Unknown)` argument type at an unrelated call).

## Reproduction

    $ mkdir -p /tmp/constpkg/pkg/src /tmp/constpkg/app/src /tmp/constpkg/app/packages

`/tmp/constpkg/pkg/project.json`

    {"name":"cst","version":"0.1.0","mfb":"1.0","kind":"package","description":"c",
     "sources":[{"root":"src","role":"package","include":["**/*.mfb"]}]}

`/tmp/constpkg/pkg/src/lib.mfb`

    EXPORT LET Answer AS Integer = 42
    EXPORT FUNC answer() AS Integer
      RETURN Answer
    END FUNC

`/tmp/constpkg/app/project.json`

    {"name":"capp","version":"0.1.0","mfb":"1.0","kind":"executable",
     "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
     "packages":[{"name":"cst","version":"=0.1.0","source":"file:packages/cst.mfp"}],
     "entry":"main","targets":["native"]}

`/tmp/constpkg/app/src/main.mfb`

    IMPORT cst
    IMPORT io

    FUNC main() AS Integer
      LET n AS Integer = cst::Answer
      io::print(toString(n))
      RETURN 0
    END FUNC

Then:

    $ mfb build /tmp/constpkg/pkg
    Wrote package to /tmp/constpkg/pkg/cst.mfp
    $ cp /tmp/constpkg/pkg/cst.mfp /tmp/constpkg/app/packages/
    $ mfb build /tmp/constpkg/app
    /tmp/constpkg/app/src/main.mfb:5 error[2-203-0043 TYPE_UNKNOWN_VALUE]: value type could not be determined
                   Initializer for binding `n` does not have a known type.

`cst::answer()` — the same value behind a FUNC — builds and prints `42`, so the
package itself and its global are fine; only the *importer's* view of the
constant is broken.

## It is a missing type, not a missing symbol

The two diagnostics differ, which localizes the gap precisely:

    LET n AS Integer = cst::Answer        -> TYPE_UNKNOWN_VALUE       (2-203-0043)
    LET n AS Integer = cst::NotEvenThere  -> SYMBOL_UNKNOWN_IDENTIFIER (2-201-0011)
                                             "Package `cst` does not export `NotEvenThere`."

So `Answer` **is** in the package's visible surface and `NotEvenThere` is not.
That surface is built in `Resolver::install_package_type_names`
(`src/resolver/packages.rs`), which unions the GLOBAL table in deliberately:

    $ grep -n "info.globals" -B 8 src/resolver/packages.rs

>   - the GLOBAL table: `EXPORT MUT` / `EXPORT LET` package state, which
>     `13_modules-and-packages.md` calls visible to importers.

    if let Ok(info) = binary_repr::read_package_info(package_file) {
        for global in info.globals {
            visible.insert(global.name);
        }
    }

`visible` is the *only* thing that consumes those rows. Nothing installs the
global's declared type for inference, and nothing lowers a read of it.

## The wire format already carries what is needed

The `.mfp` GLOBAL table records name, type, mutability and visibility —

    $ grep -n "struct BinaryReprPackageInfoGlobal" -A 6 src/binary_repr/mod.rs
    pub struct BinaryReprPackageInfoGlobal {
        pub name: String,
        pub type_: String,
        pub mutable: bool,
        pub visibility: String,
    }

— and `src/ir/package.rs` already namespaces a decoded package's globals by its
package identity (`apply_package_identity`, `rewrite_value_targets`) when merging
its IR into the consumer. So this is a front-end wiring gap over a format that
was designed for it, not a format change.

## What a fix has to cover

1. **Type installation.** `install_package_type_names` must record each exported
   global's `type_` alongside its name, and inference must type `pkg::Name` as
   that type instead of `Unknown`.
2. **Lowering.** A `pkg::Name` read must lower to a read of the namespaced merged
   global rather than nothing.
3. **Initialization order — the part that needs measuring first.** A top-level
   `LET` has an initializer. It is not established that an imported package's
   global initializers run in the consumer binary before `main`; if they do not,
   the naive fix reads a zeroed global, which is worse than the current compile
   error. **Verify this before writing the type-installation half** — a test that
   reads an imported global initialized to a non-zero constant is the whole
   check.
4. **`EXPORT MUT` too.** The same table row and the same spec sentence cover
   exported package state, which has the same gap and the added question of
   whether a consumer's write is visible to the package's own code.

Point 3 is why this is filed rather than fixed inline: the visible symptom is one
missing type, but the correctness question underneath it is initialization order
across the package boundary.

## How it was found

Writing `packages/yaml`, which wanted to export the one generator-9 error code it
raises as a constant a caller could match:

    EXPORT LET ErrExpansionLimit AS Integer = 93110001

Documented, built, and then the importer would not compile. The package now
exports `yaml::expansionLimitCode() AS Integer` instead, which works today and
would keep working if this were fixed — but a function call is not what §13
describes, and every package with a public constant has to make the same
substitution.

## Related

- `mfb spec language modules-and-packages` §13 — the specified behaviour.
- `mfb spec diagnostics error-codes`, "User-Defined Codes (generator `9`)" —
  "A handler compares against the package's own documented integer, **or against
  a constant the package exports itself**." That second option is the one this
  bug removes.
- bug-480 — the same `visible` set, which is why the *symbol* half already works.

## Point 3, measured (2026-09-06) — the initializers DO run

The report was right to gate on this. Measured before writing any of the fix, at
main `8a7735c85`, with a package exporting `MUT Counter AS Integer = 7` and a
`bump()` SUB of its own:

    $ ./build/capp.out
    7
    9

The consumer binary sees the declared `7` before `main`, and two `cst::bump()`
calls take it to `9` — so an imported package's global is a real, initialized,
shared slot in the consumer, not a zeroed one. The naive fix's failure mode does
not exist, and this is a type-installation fix, not a lowering one.

## What the fix is

1. **Type installation.** `manifest::package::imported_global_defs` decodes the
   `.mfp` GLOBAL table into `ir::ImportedGlobal { name, type_, mutable }`, keyed
   by the `package.Name` spelling a consumer reads it by. Filtered to
   `visibility == "export"`: `PRIVATE`/`PUBLIC` rows are in the same table (the
   writer records visibility in the entry flags rather than omitting the row).
   The list is threaded to the shape pass, lowering and `ir::verify` beside
   `imported_types`, and seeds `binding_types` / `globals` / `global_muts`.
2. **Lowering.** None was needed beyond naming: `ir::package::apply_package_identity`
   already rewrites both a `Global` read and an `AssignGlobal` naming
   `package.Name` to the merged `<id>.package.Name` definition. The read lowers
   under the CANONICAL name so `IMPORT pkg AS p` reaches the same slot.
4. **`EXPORT MUT` too — and a consumer's write IS visible to the package.**
   `an_exported_package_mut_is_one_shared_slot` writes `limits::Counter = 99`
   and reads it back through the package's own `limits::counter()`, which
   returns `99`. That round trip is the answer to the report's open question:
   there is one slot. An `EXPORT LET` is refused by the existing
   `TYPE_ASSIGN_REQUIRES_MUT` rule, which now has the mutability bit to read.

## A second bug this uncovered — `pkg::Name = value` was a discarded comparison

MFBASIC spells assignment and equality both `=`. `bug-468` closed the
statement-position hole for the dotted `a.b = c` spelling, but `::` is its own
token (`TokenKind::DoubleColon`), so `pkg::Name = value` matched neither that
guard nor the plain-identifier assignment arm and fell through to
`parse_expression`, where the `=` binds as EQUALITY. Before this fix the
resulting comparison could not lower (`error: NIR local reference 'cst.Counter'
does not resolve`), which accidentally hid it; typing the read would have turned
that loud internal error into a **write that compiles and silently vanishes**.
`src/ast/stmt.rs` now parses the form as an assignment, unconditionally — the
parser cannot know whether the name is a global, and a bare comparison in
statement position is never useful (bug-468's own reasoning). A name that is not
an assignable imported binding is then reported by name resolution or by the
immutable-assignment rule, both of which say what is wrong.

## Relationship to bug-554 and bug-555 — three separate causes

Reproduced all three at main `8a7735c85` before theorising. They share a
*subject* (what an importer can see of a package's exports) and nothing else:

- **bug-555** was `normalize_qualified_type_name` over-applying in a TYPE
  position (fixed on main by `93b72b92a`).
- **bug-554** is two missing membership tables plus an unqualified enum-member
  lookup in a VALUE position.
- **bug-551** is a missing global TYPE table.

Each needed its own seed or lookup; none of the three fixes makes either of the
others pass. Bisected rather than assumed: built `8f0ebfeb8^` and reproduced this
bug against it unchanged — `cst::Answer` still `TYPE_UNKNOWN_VALUE`, the write
still `error: NIR local reference 'cst.Counter' does not resolve` — so
`8f0ebfeb8` is not its ancestor either. The report's own guess that bug-554's enum-member failure was "the
same missing lookup" as this bug is disproved in bug-554's doc: the BARE enum
member already worked, so that one is about qualification, and this one is not.

Gates: full `cargo test --release --no-fail-fast` green; full
`scripts/test-accept.sh` 1416 ran / 0 mismatches; `artifact-gate.sh all` 0 diffs.
