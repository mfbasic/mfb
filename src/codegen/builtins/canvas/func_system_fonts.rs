//! `canvas::listSystemFonts`, `canvas::loadSystemFont`, and the internal
//! `canvas::systemFontTable` they read (plan-147).
//!
//! The operating system knows which fonts are installed; this build knows how to draw a
//! TrueType face. `systemFontTable` is the one native step — asking the OS — and
//! answers a single `String`: one record per face, each `name`, `postScript` and `path`
//! ended by U+001F, the record ended by U+001E. Everything else is MFBASIC: splitting
//! the table, sorting, choosing a face, and loading it through the same
//! `canvas::loadFont(path, face)` a program can call itself.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::PlatformFamily;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryHelper,
    RegistryPackage,
};
use crate::types::ParameterType;

const LIST_INTRO: &str = r#"List the full names of the fonts installed on this machine that `canvas` can draw."#;

const LIST_DESC: &str = r#"`listSystemFonts` asks the operating system which fonts are installed and returns
the full name of each one this build can draw — `"Helvetica"`, `"Helvetica Bold"`,
`"Arial Italic"` — sorted, with no name twice. Pass any of them to
`canvas::loadSystemFont`.

The list describes *this* machine. Another machine, or this one after a font is
installed or removed, can answer differently, so a program that must look the same
everywhere ships its font file and loads it with `canvas::loadFont`.

The fonts are read the first time a program asks, and the answer is kept: a font
installed while the program runs appears the next time it starts.

Only fonts with TrueType outlines are listed. Fonts in the other common format
(CFF outlines, usually `.otf` files) and variable fonts are left out, because this
build draws neither; every name in the list loads."#;

const LIST_EX: &str = r#"```
IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  FOR EACH name IN canvas::listSystemFonts()
    io::print(name)
  NEXT
END SUB
```"#;

const LOAD_INTRO: &str =
    r#"Load an installed font by its full name and hold it as a `Font` resource."#;

const LOAD_DESC: &str = r#"`loadSystemFont` finds the installed font whose full name is `name` — one of the
names `canvas::listSystemFonts` returns — and loads it exactly as `canvas::loadFont`
would, returning a `Font` bound with `RES`. The name must match exactly, including
case. A name no installed font carries fails with `ErrNotFound`.

Text drawn in a system font depends on the machine: the same name can be a different
file, or a different version of the same file, somewhere else. When the text must
look identical everywhere, ship the font file with the program and use
`canvas::loadFont` instead."#;

const LOAD_EX: &str = r#"```
IMPORT app
IMPORT canvas
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadSystemFont("Helvetica")
  LET label AS canvas::DrawItem = canvas::Text[x := 20.0, y := 60.0, text := "hello", font := face, size := 32.0, paint := canvas::fill(color::rgb(255, 255, 255))]
  canvas::present([label])
END SUB
```"#;

/// The table as a flat list of fields — `name, postScript, path, name, …` — split on
/// its UTF-8 bytes. Neither separator (U+001F, U+001E) can occur inside a multi-byte
/// UTF-8 sequence, so a byte scan finds exactly the characters the backend wrote. The
/// split is done here rather than with `strings::split` so a canvas program does not
/// carry the `strings` package for one call.
///
/// **Read once per thread, then kept.** Asking the operating system is the expensive
/// part — measured 0.58 s per call on macOS (20 `listSystemFonts` calls in 11.5 s),
/// against 0.08 s for loading the font itself — and a program that loads four fonts at
/// start-up would otherwise pay it four times. A font installed while the program runs
/// therefore appears the next time it starts; the man pages say so.
#[rustfmt::skip]
const SYSTEM_FONT_FIELDS: &str =
r#"MUT __CANVAS_SYSFONT_READ AS Boolean = FALSE
MUT __CANVAS_SYSFONT_FIELDS AS List OF String = []

FUNC __canvas_systemFontFields() AS List OF String
  IF NOT __CANVAS_SYSFONT_READ THEN
    __CANVAS_SYSFONT_FIELDS = __canvas_readSystemFontFields()
    __CANVAS_SYSFONT_READ = TRUE
  END IF
  RETURN __CANVAS_SYSFONT_FIELDS
END FUNC

FUNC __canvas_readSystemFontFields() AS List OF String
  LET bytes AS List OF Byte = encoding::utf8Encode(canvas::systemFontTable())
  MUT fields AS List OF String = []
  MUT start AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < len(bytes)
    LET b AS Integer = toInt(collections::getOr(bytes, i, toByte(0)))
    IF b = 30 OR b = 31 THEN
      fields = collections::append(fields, encoding::utf8Decode(collections::mid(bytes, start, i - start)))
      start = i + 1
    END IF
    i = i + 1
  END WHILE
  RETURN fields
END FUNC"#;

/// The two public members over the table. `loadSystemFont` sorts candidates by path
/// then PostScript name, so when two files carry the same full name — a user copy and
/// a system copy — the choice is the same on every run. The face is named to the loader
/// by PostScript name, which is unambiguous; a backend that could not report one leaves
/// it empty, and the full name is used instead.
#[rustfmt::skip]
const LIST_SYSTEM_FONTS: &str =
r#"FUNC __canvas_listSystemFonts() AS List OF String
  LET fields AS List OF String = __canvas_systemFontFields()
  MUT names AS List OF String = []
  MUT i AS Integer = 0
  WHILE i + 2 < len(fields)
    names = collections::append(names, collections::getOr(fields, i, ""))
    i = i + 3
  END WHILE
  RETURN __canvas_sortedUnique(names)
END FUNC"#;

/// Sort a list of names and drop repeats — a merge sort, because `collections::sort`
/// and `collections::distinct` are MFBASIC-source generics that reach a program only
/// when *the program* imports `collections`: a companion calling them links against a
/// function the build never injected (see the registry test
/// `no_companion_calls_a_source_generic_collections_member`).
#[rustfmt::skip]
const SORTED_UNIQUE: &str =
r#"FUNC __canvas_sortedUnique(names AS List OF String) AS List OF String
  IF len(names) <= 1 THEN
    RETURN names
  END IF
  LET half AS Integer = len(names) / 2
  LET left AS List OF String = __canvas_sortedUnique(collections::mid(names, 0, half))
  LET right AS List OF String = __canvas_sortedUnique(collections::mid(names, half, len(names) - half))
  MUT out AS List OF String = []
  MUT i AS Integer = 0
  MUT j AS Integer = 0
  WHILE i < len(left) OR j < len(right)
    MUT pick AS String = ""
    IF j >= len(right) THEN
      pick = collections::getOr(left, i, "")
      i = i + 1
    ELSE
      IF i < len(left) AND collections::getOr(left, i, "") <= collections::getOr(right, j, "") THEN
        pick = collections::getOr(left, i, "")
        i = i + 1
      ELSE
        pick = collections::getOr(right, j, "")
        j = j + 1
      END IF
    END IF
    IF len(out) = 0 OR collections::getOr(out, len(out) - 1, "") <> pick THEN
      out = collections::append(out, pick)
    END IF
  END WHILE
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const LOAD_SYSTEM_FONT: &str =
r#"FUNC __canvas_loadSystemFont(name AS String) AS canvas::Font
  LET fields AS List OF String = __canvas_systemFontFields()
  MUT found AS Boolean = FALSE
  MUT bestKey AS String = ""
  MUT bestPath AS String = ""
  MUT bestFace AS String = ""
  MUT i AS Integer = 0
  WHILE i + 2 < len(fields)
    IF collections::getOr(fields, i, "") = name THEN
      LET postScript AS String = collections::getOr(fields, i + 1, "")
      LET path AS String = collections::getOr(fields, i + 2, "")
      LET key AS String = path & "/" & postScript
      IF NOT found OR key < bestKey THEN
        found = TRUE
        bestKey = key
        bestPath = path
        bestFace = postScript
      END IF
    END IF
    i = i + 3
  END WHILE
  IF NOT found THEN
    ' 77050004 is errorCode.ErrNotFound, spelled as a literal because the injected
    ' builtin source does not IMPORT errorCode.
    FAIL error(77050004, "no system font named: " & name)
  END IF
  IF bestFace = "" THEN
    bestFace = name
  END IF
  RETURN canvas::loadFont(bestPath, bestFace)
END FUNC"#;

/// `canvas::systemFontTable()` — the per-OS enumeration. Each backend emits the whole
/// body and leaves the `String` in the result registers.
pub(crate) fn lower_system_font_table(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    match ctx.platform.family() {
        PlatformFamily::MacOS => super::gen_system_fonts_macos::emit_system_font_table(builder, ctx)?,
        PlatformFamily::Linux => {
            return Err("canvas.systemFontTable has no linux backend".to_string())
        }
        PlatformFamily::Windows => {
            return Err("canvas.systemFontTable has no windows backend".to_string())
        }
    }
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.systemFontTable".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "canvas_systemFontFields",
        SYSTEM_FONT_FIELDS,
    ));
    pkg.add_helper(RegistryHelper::always(
        "canvas_sortedUnique",
        SORTED_UNIQUE,
    ));
    pkg.add_function(RegistryFunction {
        name: "systemFontTable",
        intro: "The installed fonts this build can draw, as one delimited table.",
        desc: "Internal: one record per face — full name, PostScript name and file \
               path, each ended by U+001F, the record ended by U+001E — from CoreText, \
               fontconfig or DirectWrite. `listSystemFonts` and `loadSystemFont` \
               read it.",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_system_font_table),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "listSystemFonts",
        intro: LIST_INTRO,
        desc: LIST_DESC,
        example: LIST_EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec!["ErrOutOfMemory"],
            body: Body::mfb(LIST_SYSTEM_FONTS, "__canvas_listSystemFonts"),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "loadSystemFont",
        intro: LOAD_INTRO,
        desc: LOAD_DESC,
        example: LOAD_EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc: "The font's full name, as `canvas::listSystemFonts` spells it.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::FONT_TYPE_ID),
            errors: vec!["ErrNotFound", "ErrBadFontFile", "ErrOutOfMemory"],
            body: Body::mfb(LOAD_SYSTEM_FONT, "__canvas_loadSystemFont"),
        }],
    });
}
