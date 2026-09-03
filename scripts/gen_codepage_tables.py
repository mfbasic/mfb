#!/usr/bin/env python3
"""Generate `src/codegen/builtins/encoding/helper_codepage_table.rs` (plan-123).

Reads the vendored WHATWG legacy single-byte index files under
`tools/codepage-index/` and emits the `Codepage` enum's variant list together with
the `__encoding_codepageTable` MFBASIC body, so the two cannot drift apart.

Each table is one 128-scalar MFBASIC String literal: scalar `i` is the code point
for byte `128 + i`, and `\\u{FFFD}` marks a byte the codepage leaves unmapped.
U+FFFD is an unambiguous sentinel because the highest code point across all 27
index files is U+FB02 (`scripts/audit_codepage_index.py`).

Writes the artifact to **stdout** and its stats to stderr, per the
`scripts/check-generated.sh` contract:

    python3 scripts/gen_codepage_tables.py > src/codegen/builtins/encoding/helper_codepage_table.rs

`scripts/check-generated.sh` re-runs it and fails on drift, so a hand edit of the
artifact cannot land.
"""

import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INDEX_DIR = os.path.join(ROOT, "tools", "codepage-index")
SENTINEL = 0xFFFD

# (variant name, index-file label or None, `mfb man` description). ORDER IS A
# COMPATIBILITY SURFACE -- it fixes each variant's discriminant. Append only; never
# reorder or remove.
#
# `Utf8` has no index file: it decodes and encodes through the existing UTF-8
# codecs, so a caller holding a charset label has one entry point for the common
# case too. `Iso8859_8I` has no index file of its own (upstream serves HTTP 404 for
# it) and shares ISO-8859-8's table, which is the standard's own position -- the two
# differ only in bidi display direction, not in the byte<->code-point mapping.
CODEPAGES = [
    ("Utf8", None,
     "UTF-8. Not a single-byte codepage: decodes and encodes through the UTF-8 codecs, so one call site can handle UTF-8 and the legacy codepages alike."),
    ("Ibm866", "ibm866",
     "IBM866 -- Cyrillic, the Russian MS-DOS codepage."),
    ("Iso8859_2", "iso-8859-2",
     "ISO-8859-2 (Latin-2) -- Central and Eastern European."),
    ("Iso8859_3", "iso-8859-3",
     "ISO-8859-3 (Latin-3) -- South European, Maltese and Esperanto. Leaves 7 high bytes undefined."),
    ("Iso8859_4", "iso-8859-4",
     "ISO-8859-4 (Latin-4) -- North European and Baltic."),
    ("Iso8859_5", "iso-8859-5",
     "ISO-8859-5 -- Cyrillic."),
    ("Iso8859_6", "iso-8859-6",
     "ISO-8859-6 -- Arabic. Defines only 83 of its 128 high bytes."),
    ("Iso8859_7", "iso-8859-7",
     "ISO-8859-7 -- Greek. Leaves 3 high bytes undefined."),
    ("Iso8859_8", "iso-8859-8",
     "ISO-8859-8 -- Hebrew, visual order. Defines only 92 of its 128 high bytes."),
    ("Iso8859_8I", "iso-8859-8",
     "ISO-8859-8-I -- Hebrew, logical order. The same byte mapping as `Iso8859_8`; the two differ only in display direction."),
    ("Iso8859_10", "iso-8859-10",
     "ISO-8859-10 (Latin-6) -- Nordic."),
    ("Iso8859_13", "iso-8859-13",
     "ISO-8859-13 (Latin-7) -- Baltic Rim."),
    ("Iso8859_14", "iso-8859-14",
     "ISO-8859-14 (Latin-8) -- Celtic."),
    ("Iso8859_15", "iso-8859-15",
     "ISO-8859-15 (Latin-9) -- Western European, with the euro sign in place of the currency sign."),
    ("Iso8859_16", "iso-8859-16",
     "ISO-8859-16 (Latin-10) -- South-Eastern European."),
    ("Koi8R", "koi8-r",
     "KOI8-R -- Russian Cyrillic."),
    ("Koi8U", "koi8-u",
     "KOI8-U -- Ukrainian Cyrillic."),
    ("Macintosh", "macintosh",
     "Mac OS Roman -- the classic Macintosh Western character set."),
    ("Windows874", "windows-874",
     "windows-874 -- Thai. Defines 120 of its 128 high bytes."),
    ("Windows1250", "windows-1250",
     "windows-1250 -- Central European."),
    ("Windows1251", "windows-1251",
     "windows-1251 -- Cyrillic."),
    ("Windows1252", "windows-1252",
     "windows-1252 -- Western European. The most common legacy web encoding."),
    ("Windows1253", "windows-1253",
     "windows-1253 -- Greek. Leaves 3 high bytes undefined."),
    ("Windows1254", "windows-1254",
     "windows-1254 -- Turkish."),
    ("Windows1255", "windows-1255",
     "windows-1255 -- Hebrew. Defines 118 of its 128 high bytes."),
    ("Windows1256", "windows-1256",
     "windows-1256 -- Arabic."),
    ("Windows1257", "windows-1257",
     "windows-1257 -- Baltic. Leaves 2 high bytes undefined."),
    ("Windows1258", "windows-1258",
     "windows-1258 -- Vietnamese."),
    ("MacCyrillic", "x-mac-cyrillic",
     "Mac OS Cyrillic -- the classic Macintosh Cyrillic character set."),
]

TEMPLATE = '''//! `Codepage` and `__encoding_codepageTable` -- GENERATED, DO NOT EDIT BY HAND.
//!
//! Written by `scripts/gen_codepage_tables.py` from the vendored WHATWG legacy
//! single-byte index files under `tools/codepage-index/`:
//!
//! ```text
//! python3 scripts/gen_codepage_tables.py > src/codegen/builtins/encoding/helper_codepage_table.rs
//! ```
//!
//! `scripts/check-generated.sh` re-runs the generator and fails on drift, so a hand
//! edit here is a CI failure rather than a silent divergence from upstream.
//!
//! This one file owns both the `Codepage` enum's variant list and the `MATCH` arms
//! that map a variant to its table, so a variant can never exist without a table nor
//! a table without a variant. [`VARIANTS`] also carries each variant's index-file
//! label, which is what lets `codepage_tables_match_the_vendored_index_files`
//! (in the parent module) check every scalar of every table against
//! `tools/codepage-index/` at test time rather than trusting this file.
//!
//! Each table is one 128-scalar MFBASIC `String` literal: scalar `i` is the code
//! point for byte `128 + i`, and `\\u{{FFFD}}` marks a byte the codepage leaves
//! unmapped. U+FFFD is an unambiguous sentinel because the highest code point
//! across all 27 index files is U+FB02 (`scripts/audit_codepage_index.py`).
//!
//! The enum renders ahead of the helpers (records -> unions -> enums -> helpers ->
//! member bodies), so the body below can `MATCH` on it. Variant ORDER fixes each
//! discriminant and is a compatibility surface: append only, never reorder or
//! remove.
//!
//! Body byte-significant (2-space indent -> `.ncode` columns); do not reformat.

use crate::codegen::registry::{{EnumVariant, RegistryEnum, RegistryHelper, RegistryPackage}};

/// Every `Codepage` variant in discriminant order, as
/// `(variant name, index-file label, description)`. The label is the
/// `tools/codepage-index/index-<label>.txt` stem the variant's table comes from, or
/// `""` for a variant with no single-byte table (`Utf8`).
pub(super) const VARIANTS: &[(&str, &str, &str)] = &[
{variants}];

#[rustfmt::skip]
pub(super) const BODY: &str =
r#"' The 128-scalar high-half table for a single-byte codepage. Scalar i is the code
' point for byte 128 + i; "\\u{{FFFD}}" marks a byte this codepage leaves unmapped.
' `Codepage.Utf8` has no table -- `codepageDecode` and `codepageEncode` branch on it
' before they get here -- so it answers with the empty string.
FUNC __encoding_codepageTable(codepage AS Codepage) AS String
  MATCH codepage
{arms}  END MATCH
END FUNC"#;

/// Register the `Codepage` enum and its table helper. Both are driven by
/// [`VARIANTS`] and [`BODY`], which the generator emits together.
pub(crate) fn register(pkg: &mut RegistryPackage) {{
    pkg.add_enum(RegistryEnum {{
        name: "Codepage",
        export: true,
        variants: VARIANTS
            .iter()
            .map(|(name, _label, description)| EnumVariant {{
                name,
                description,
                advisory: None,
            }})
            .collect(),
    }});
    pkg.add_helper(RegistryHelper::always("encoding_codepageTable", BODY));
}}
'''


def read_index(label):
    """Return {pointer: codepoint} for one vendored index file."""
    path = os.path.join(INDEX_DIR, f"index-{label}.txt")
    rows = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            text = line.split("#")[0].strip()
            if not text:
                continue
            fields = text.split()
            rows[int(fields[0])] = int(fields[1], 16)
    return rows


def literal(label):
    """Render one index file as a 128-scalar MFBASIC string literal."""
    rows = read_index(label)
    top = max(rows.values())
    if top >= SENTINEL:
        raise SystemExit(
            f"index-{label}.txt maps U+{top:04X}, which collides with the "
            f"U+{SENTINEL:04X} hole sentinel"
        )
    return "".join("\\u{%04X}" % rows.get(i, SENTINEL) for i in range(128))


def rust_str(text):
    """Render `text` as a Rust string literal."""
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def render():
    variants = "".join(
        "    ({}, {}, {}),\n".format(
            rust_str(name), rust_str(label or ""), rust_str(desc)
        )
        for name, label, desc in CODEPAGES
    )
    arms = "".join(
        f"    CASE Codepage.{name}\n"
        f'      RETURN "{literal(label) if label else ""}"\n'
        for name, label, _desc in CODEPAGES
    )
    return TEMPLATE.format(variants=variants, arms=arms)


def main() -> int:
    text = render()
    sys.stdout.write(text)
    tables = len({label for _n, label, _d in CODEPAGES if label})
    print(
        f"{len(CODEPAGES)} variants over {tables} tables, {len(text)} bytes",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
