#!/usr/bin/env python3
"""plan-145-A: generate `field_kinds.tsv` — one line per record-field KIND.

    python3 tests/runtime/inplace_self_update/field_kinds_gen.py target/debug/mfb > \
        tests/runtime/inplace_self_update/field_kinds.tsv

`cases.tsv` covers collection fields (every self-update-shaped collection overload).
This file covers every other kind a record field can have: the builtin scalars,
`String`/`AttributedString`, `json::Json`, a fixed-size and a variable-size user
record, and **every record type on every `mfb man <pkg> types` Records section**
(the black-box census `tests/guards/inplace_self_update_census.rs` holds this file
to that list).

The rows in plan-144's F2–F5 families differ by builtin but not by lowering (no arm
names them, findings Appendix B.3), so one representative self-update per kind
measures the kind's path.

A kind's class is the compiler's (`field_kind_class`, `self_update.rs`, checked by
the unit census against `FIELD_KIND_TABLE`), recomputed here from the `mfb man`
field lists with the same rules:

  Scalar           a builtin scalar, an enum, `Nothing`, a resource handle
  Pointer          holds a `json::Json` (not memcpy-copyable, so not inlined)
  InlinedFixed     every field Scalar or InlinedFixed
  InlinedVariable  anything else inlined (a `String`, a collection, a data union)

and its expectation per site from the plan-145 letter that lands it
(plan-145-A §"plan-145 as a whole", Open Decisions 1, 3 and 4):

  Scalar           S3 S4 S10 -> copy:C, S5 -> copy:G, S6 T6 -> copy:F, S9 -> copy:H,
                   T1-T5 T8 -> arm (Layer 1 today)
  Pointer          S3 S4 S10 T1-T5 T8 -> copy:C, S5 -> copy:G, S6 T6 -> copy:F,
                   S9 -> copy:H; `json::Json` is no STATE type -> na at T sites
  InlinedFixed     every site -> copy:F, except S5 -> copy:G and S9 -> copy:H
  InlinedVariable  rebuild:size-varies (Open Decision 3)
  String kinds     deferred:string (Open Decision 1)

and S7/T7 are `na:TYPE_FOR_EACH_REQUIRES_COLLECTION` for every kind (none is a
collection). A `canvas::` kind builds in app mode (`build` column `app`).

Columns: kind \t build \t setup \t statement \t check \t bound \t expects
  setup      `<type>` (a defaultable type: `MUT x AS <type>`) or `<type> = <init>`
  statement  `x = …`, run each iteration
  check      a String expression over `before` (a copy of the field)
  bound      `arm` or `arm+value` (the new value allocates on its own)
  expects    `site=expect` for all 15 field sites, space-separated
"""
import re
import subprocess
import sys

MFB = sys.argv[1]
SITES = ["S3", "S4", "S5", "S6", "S7", "S9", "S10",
         "T1", "T2", "T3", "T4", "T5", "T6", "T7", "T8"]
NA_LOOP = "na:TYPE_FOR_EACH_REQUIRES_COLLECTION"
PRINTABLE = {"Integer", "Float", "Fixed", "Money", "Boolean", "Byte"}


def man(*args):
    return subprocess.run([MFB, "man", *args], capture_output=True, text=True).stdout


# ---- every package's record types, unions and enums, from `mfb man <pkg> types` ----
pkgs = sorted({m.group(1) for line in man().splitlines()
               if (m := re.match(r"│ (\w+)\s+│", line)) and m.group(1) not in ("Package", "Topic")})
records, unions, enums, order = {}, set(), set(), []
for p in pkgs:
    lines = man(p, "types").splitlines()
    section = cur = None
    for i, line in enumerate(lines):
        if i + 1 < len(lines) and lines[i + 1].startswith("─") and line.strip():
            section, cur = line.strip(), None
            continue
        if re.match(r"^\w+::\w+$", line.strip()):
            name = line.strip()
            if section == "Records":
                cur = name
                records[cur] = []
                order.append(cur)
            elif section == "Unions":
                unions.add(name)
            elif section == "Enums":
                enums.add(name)
            continue
        m = re.match(r"│ (\w+)\s+│ (.+?)\s+│", line)
        if section == "Records" and cur and m and m.group(1) != "Field":
            records[cur].append((m.group(1), m.group(2), p))

# Qualify a bare in-package type name anywhere in a field type (`List OF Json`).
declared = set(records) | unions | enums
for rec, fields in records.items():
    records[rec] = [(name, re.sub(r"(?<![:\w])([A-Z]\w*)",
                                  lambda m, p=p: f"{p}::{m.group(1)}"
                                  if f"{p}::{m.group(1)}" in declared else m.group(1), t))
                    for name, t, p in fields]

# The user records the census adds beside the package ones (the harness declares
# them: `kind_helpers`).
records["KFix"] = [("p", "Integer"), ("q", "Float")]
records["KVar"] = [("s", "String")]


def cls(t, seen=()):
    if t in PRINTABLE or t in enums or t == "Nothing" or t.startswith("RES "):
        return "Scalar"
    if t == "json::Json":
        return "Pointer"
    if t == "String" or t in unions or t.split()[0] in ("List", "Map", "Set"):
        # A collection or union is copyable exactly when what it holds is.
        return "Pointer" if "json::Json" in t else "InlinedVariable"
    if t.startswith("FUNC"):
        return "InlinedVariable"
    if t in records:
        if t in seen:
            return "InlinedVariable"
        ks = [cls(ft, seen + (t,)) for _, ft in records[t]]
        if "Pointer" in ks:
            return "Pointer"
        return "InlinedFixed" if all(k in ("Scalar", "InlinedFixed") for k in ks) else "InlinedVariable"
    sys.exit(f"unclassified field type {t}")


def check_expr(t, path="before"):
    """A String rendering of one field reachable from `path` (of type `t`): a
    printable scalar or `String` first, then a collection's length, then a nested
    record of the SAME package (another package's fields need its `IMPORT`)."""
    for name, ft in records.get(t, []):
        if ft in PRINTABLE:
            return f"toString({path}.{name})"
        if ft == "String":
            return f"{path}.{name}"
    for name, ft in records.get(t, []):
        if ft.split()[0] in ("List", "Map", "Set"):
            return f"toString(len({path}.{name}))"
    for name, ft in records.get(t, []):
        if ft in records and ft.split("::")[0] == t.split("::")[0]:
            inner = check_expr(ft, f"{path}.{name}")
            if inner:
                return inner
    return None


# Types a `MUT x AS K` cannot declare (TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE), with a value.
LITERALS = {
    "astrings::AttrFlag": "astrings::AttrFlag[kind := astrings::AttrTypeFlag.Bold]",
    "astrings::AttrNumber": "astrings::AttrNumber[kind := astrings::AttrTypeNumber.FontSize, value := 12]",
    "astrings::AttrText": 'astrings::AttrText[kind := astrings::AttrTypeText.Font, value := "Mono"]',
    "http::Route": 'http::Route[pattern := "/", handler := kindRoute]',
    # `net::Address` is compiler-owned (no literal) but defaultable: `kindAddress()`.
    "net::PingResult": "net::PingResult[status := net::PingStatus.Ok, address := kindAddress(), rttMs := 1.0, ttl := 1, size := 1]",
    "term::MouseEvent": "term::MouseEvent[kind := term::MouseKind.None, button := term::MouseButton.None, row := 1, column := 1, shift := FALSE, ctrl := FALSE, alt := FALSE]",
    # The canvas records that hold an enum or a `canvas::Paint` (`canvas::fill`).
    "canvas::Paint": "canvas::fill(color::fromName(\"red\"))",
    "canvas::MouseEvent": "canvas::MouseEvent[kind := canvas::MouseKind.None, button := canvas::MouseButton.None, position := canvas::Point[0.0, 0.0], shift := FALSE, ctrl := FALSE, alt := FALSE]",
    "canvas::Gradient": "canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[0.0, 0.0], endPoint := canvas::Point[1.0, 1.0], stops := []]",
    "canvas::Rectangle": "canvas::Rectangle[x := 1.0, y := 1.0, w := 2.0, h := 2.0, paint := canvas::fill(color::fromName(\"red\"))]",
    "canvas::RoundedRect": "canvas::RoundedRect[x := 1.0, y := 1.0, w := 2.0, h := 2.0, cornerRadius := 0.5, paint := canvas::fill(color::fromName(\"red\"))]",
    "canvas::Circle": "canvas::Circle[x := 1.0, y := 1.0, radius := 2.0, paint := canvas::fill(color::fromName(\"red\"))]",
    "canvas::Ellipse": "canvas::Ellipse[x := 1.0, y := 1.0, radiusX := 2.0, radiusY := 1.0, angle := 0.0, paint := canvas::fill(color::fromName(\"red\"))]",
    "canvas::Line": "canvas::Line[x1 := 0.0, y1 := 0.0, x2 := 1.0, y2 := 1.0, cap := canvas::CapStyle.Butt, paint := canvas::fill(color::fromName(\"red\"))]",
    "canvas::Arc": "canvas::Arc[x := 1.0, y := 1.0, radius := 2.0, startAngle := 0.0, endAngle := 1.0, cap := canvas::CapStyle.Butt, paint := canvas::fill(color::fromName(\"red\"))]",
    "canvas::Polygon": "canvas::Polygon[points := [canvas::Point[0.0, 0.0]], paint := canvas::fill(color::fromName(\"red\"))]",
    "KFix": "KFix[p := 1, q := 2.5]",
    "KVar": 'KVar[s := "v"]',
}


# Kinds whose values the harness's forms cannot build at all: they hold a live
# `RES` handle (a font, an image), so `MUT x AS K` has no default and no literal
# can name the resource.
UNBUILDABLE = {"canvas::Text", "canvas::Picture"}


def expects(klass, state_ok=True, buildable=True):
    out = {}
    for s in SITES:
        if not buildable:
            out[s] = "na:TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE"
        elif s.startswith("T") and not state_ok:
            # A `STATE` payload must be defaultable (`TYPE_STATE_INVALID`).
            out[s] = "na:TYPE_STATE_INVALID"
        elif s in ("S7", "T7"):
            out[s] = NA_LOOP
        elif klass == "String":
            out[s] = "deferred:string"
        elif klass == "InlinedVariable":
            out[s] = "rebuild:size-varies"
        elif s == "S5":
            out[s] = "copy:G"
        elif s == "S9":
            out[s] = "copy:H"
        elif s in ("S6", "T6"):
            out[s] = "copy:F"
        elif klass == "InlinedFixed":
            out[s] = "copy:F"
        elif klass == "Scalar":
            out[s] = "arm" if s.startswith("T") else "copy:C"
        elif klass == "Pointer":
            out[s] = "copy:C"
    return " ".join(f"{s}={out[s]}" for s in SITES)


lines = []


def emit(kind, build, setup, statement, check, bound, klass, state_ok=True, buildable=True):
    lines.append("\t".join([kind, build, setup, statement, check, bound,
                             expects(klass, state_ok, buildable)]))


print("# plan-145-A: one line per record-field kind that cases.tsv does not cover.")
print("# Generated by field_kinds_gen.py (see it for the columns and the rules); a letter")
print("# that lands a kind flips its sites by hand.")
print("# Columns: kind \\t build \\t setup \\t statement \\t check \\t bound \\t expects")
emit("Integer", "console", "Integer = 5", "x = x + 1", "toString(before)", "arm", "Scalar")
emit("Float", "console", "Float = 1.5", "x = x + 0.5", "toString(before)", "arm", "Scalar")
emit("Fixed", "console", "Fixed = 1.5F", "x = x + 0.5F", "toString(before)", "arm", "Scalar")
emit("Money", "console", "Money = 1.25m", "x = x + 0.25m", "toString(before)", "arm", "Scalar")
emit("Boolean", "console", "Boolean = TRUE", "x = NOT x", "toString(before)", "arm", "Scalar")
# The `Byte` statement allocates on its own (`toString`), so its bound is `arm+value`.
emit("Byte", "console", "Byte = toByte(1)", "x = toByte(len(toString(x)))",
     "toString(before)", "arm+value", "Scalar")
emit("String", "console", 'String = "a"', 'x = x & "b"', "before", "arm", "String")
emit("AttributedString", "console", 'AttributedString = astrings::fromString("a")',
     "x = astrings::clearAttributes(x)", "astrings::toMarkdown(before)", "arm", "String")
emit("json::Json", "console", 'json::Json = json::parse("[1]")',
     "x = kindJson(x)", "json::stringify(before)", "arm+value", "Pointer", state_ok=False)
for kind in order + ["KFix", "KVar"]:
    klass = cls(kind)
    build = "app" if kind.startswith("canvas::") else "console"
    setup = f"{kind} = {LITERALS[kind]}" if kind in LITERALS else kind
    if kind.startswith("vector::"):
        statement = "x = vector::max(x, x)"
    elif klass == "Pointer":
        # A pointer field stores a value it owns; one borrowed from `x` would be
        # deep-copied whatever the lowering (see `kindFresh` in the harness).
        statement = "x = kindFresh(x)"
    else:
        statement = "x = kindSame(x)"
    check = check_expr(kind) or '"-"'
    bound = "arm+value" if klass in ("InlinedFixed", "Pointer") else "arm"
    # A type with no default (it needed a literal) cannot be a `STATE` payload
    # field; the user records are defaultable and use a literal only for a value.
    state_ok = kind not in LITERALS or kind in ("KFix", "KVar")
    emit(kind, build, setup, statement, check, bound, klass, state_ok, kind not in UNBUILDABLE)
print("\n".join(lines))
