# bug-554: an imported package's UNION and ENUM lose their members, so every `MATCH` on one is "not exhaustive"

Last updated: 2026-09-05
Effort: medium (resolver + the exhaustiveness checker; the wire format already carries what is needed)
Severity: MEDIUM (a specified language feature is unusable across a package boundary; a package that exports a union cannot be consumed)
Class: Unimplemented spec surface

Status: Open

## The finding

`mfb spec language modules-and-packages` (§13) says a top-level `UNION` and
`ENUM` may be `EXPORT`ed, and that the qualified-name rule covers their members:

> Top-level LET, MUT, FUNC, SUB, TYPE, UNION, ENUM, and RESOURCE may use
> PRIVATE, PUBLIC, or EXPORT.

> This holds for every kind of name alike: variables and constants, functions,
> records, **unions, union variants, enums, enum members**, and resource types.

The TYPE NAMES cross the boundary — `resolver::packages::install_package_type_names`
inserts the union, each variant, and the enum — but the MEMBERSHIP does not. The
importer learns that `Item`, `Note`, `Tally` and `Colour` are types; it does not
learn that `Item`'s variants are `Note` and `Tally`, or that `Colour`'s members
are `Red` and `Green`. Every `MATCH` on an imported union or enum is therefore
reported as covering nothing:

    TYPE_MATCH_NOT_EXHAUSTIVE  match cases do not cover every possible value

There is no spelling that works. Bare and qualified fail identically, and
`CASE ELSE` only converts the compile error into a program that cannot
distinguish the variants it was written to distinguish.

This is why a `json::Json`-shaped API can be written by a BUILT-IN package and
not by a source one: `json`'s union is compiled into the importer's own project,
so its membership is present. An imported `.mfp`'s is not.

## Reproduction

    $ mkdir -p /tmp/unionpkg/pkg/src /tmp/unionpkg/app/src

`/tmp/unionpkg/pkg/project.json`

    {"name":"recpkg","version":"0.1.0","mfb":"1.0","kind":"package","description":"u",
     "sources":[{"root":"src","role":"package","include":["**/*.mfb"]}]}

`/tmp/unionpkg/pkg/src/lib.mfb`

    EXPORT TYPE Note
      label AS String
    END TYPE

    EXPORT TYPE Tally
      count AS Integer
    END TYPE

    EXPORT UNION Item
      Note
      Tally
    END UNION

    EXPORT ENUM Colour
      Red, Green
    END ENUM

    EXPORT FUNC anItem() AS Item
      RETURN Note[label := "in a union"]
    END FUNC

    EXPORT FUNC aColour() AS Colour
      RETURN Colour.Green
    END FUNC

`/tmp/unionpkg/app/project.json`

    {"name":"recapp","version":"0.1.0","mfb":"1.0","kind":"executable","description":"u",
     "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
     "packages":[{"name":"recpkg","version":"=0.1.0","source":"file:../pkg"}],
     "entry":"main","targets":["native"]}

`/tmp/unionpkg/app/src/main.mfb`

    IMPORT recpkg
    IMPORT io

    FUNC main() AS Integer
      LET item AS recpkg::Item = recpkg::anItem()
      MATCH item
        CASE recpkg::Note(n)  : io::print(n.label)
        CASE recpkg::Tally(t) : io::print(toString(t.count))
      END MATCH
      RETURN 0
    END FUNC

Then:

    $ mfb build /tmp/unionpkg/app
    /tmp/unionpkg/app/src/main.mfb:6 error[2-203-0062 TYPE_MATCH_NOT_EXHAUSTIVE]:
      match cases do not cover every possible value

The bare spelling (`CASE Note(n)`, `LET item AS Item`) fails identically, and so
does the enum:

    MATCH c
      CASE recpkg::Colour.Red   : io::print("red")
      CASE recpkg::Colour.Green : io::print("green")
    END MATCH
    ' same TYPE_MATCH_NOT_EXHAUSTIVE

An enum member used as a VALUE fails one step earlier, with
`TYPE_UNKNOWN_VALUE` on `recpkg::Colour.Red` — the same shape as bug-551's
constants, and probably the same missing lookup.

## What is and is not affected

Reading a RECORD's fields across the boundary works, including under a qualified
type annotation and in a parameter position. That path was broken in the same
way until the `normalize_qualified_type_name` change landed with
`packages/json_schema`; unions and enums were untouched by it and are the
remaining half.

A package may therefore export records and functions freely today, and cannot
export a usable union or enum at all.

## Where to look

* `src/resolver/packages.rs:install_package_type_names` — reads
  `binary_repr::read_package_type_exports`, which already carries
  `export.variants`. It inserts each variant's NAME into `self.types` and drops
  the association with the union.
* Whatever populates the union-membership and enum-member tables the
  exhaustiveness check consults — the same tables a local `UNION`/`ENUM`
  declaration fills.
* `mfb spec package` — confirm the `.mfp` sections already carry the membership
  (the type-export table's `variants` suggests they do, so this is a front-end
  plumbing gap and not a wire-format change).

## Why it matters

It is the difference between a package that can define a data model and one that
can only define functions over somebody else's. Every sum type — a parsed
document, a result classification, a state machine — is a union, and today none
of them can be exported.
