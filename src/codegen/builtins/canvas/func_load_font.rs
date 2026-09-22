//! `canvas::loadFont` and its constructor `canvas::fontFromBytes`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryHelper,
    RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_font::FONT_BYTES;

const INTRO: &str = r#"Load a TrueType font file and hold it as a `Font` resource."#;

const DESC: &str = r#"`loadFont` reads the file at `path` and returns a `Font`, bound with `RES`. It closes
by itself when it leaves scope, and `canvas::destroyFont` closes it sooner.
A `canvas::Text` item holds the font itself. You can still close a font while a scene
that draws with it is on screen: the item then draws nothing rather than failing.

**The file is kept whole, not decoded.** A TrueType file *is* the glyph database —
its `loca` table indexes `glyf` by glyph id — so decoding up front would mean
deciding in advance which glyphs the program will draw. Glyph outlines are read on
demand and the rasterised result is cached, which is where repeated work is actually
saved.

**What this build reads.** TrueType outlines: an sfnt whose version is `0x00010000`
or the tag `true`. A font collection (a `.ttc` file, several faces in one) loads its
first face; pass `face` to choose another by its PostScript name (`Helvetica-Bold`) or
its full name (`Helvetica Bold`). A `face` the file does not carry is `ErrNotFound`.
It refuses CFF/OpenType-PostScript outlines and WOFF with
`ErrBadFontFile` — a different mistake from a path that does not exist, and it needs a
different fix. A TrueType file
whose `head` table puts `unitsPerEm` outside the 16..16384 the format allows is
refused the same way. When text is drawn, a glyph whose bitmap at the requested size
would exceed 8192 pixels a side or 16,777,216 pixels draws nothing.

There is no font *discovery*: `path` names a file. A program that wants a system
font names its path, which keeps the rendering reproducible — the same file produces
the same pixels on every platform, which is what makes text goldens exact-match."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("DejaVuSans.ttf")
  LET label AS canvas::DrawItem = canvas::Text[x := 20.0, y := 60.0, text := "hello", font := face, size := 32.0, paint := canvas::fill(color::rgb(255, 255, 255))]
  canvas::present([label])
END SUB
```"#;

/// `canvas::loadFont(path)` — read the file, check it is TrueType, hand it on.
///
/// MFBASIC rather than an emitter, because every step is a call it can already make:
/// `fs::readBytes` for the file, `collections::getOr` for the four version bytes, and
/// `canvas::fontFromBytes` for the one thing MFBASIC cannot do — stamp a resource
/// record. Splitting it that way keeps the *rule* about which files are acceptable
/// readable, instead of spelling it in loads and compares.
#[rustfmt::skip]
const LOAD_FONT: &str =
r#"FUNC __canvas_loadFont(path AS String) AS canvas::Font
  LET bytes AS List OF Byte = fs::readBytes(path)
  MUT dir AS Integer = 0
  IF __canvas_isCollection(bytes) THEN
    LET faces AS List OF Integer = __canvas_sfntFaces(bytes)
    IF len(faces) = 0 THEN
      ' 77050022 is errorCode.ErrBadFontFile, spelled as a literal for the reason
      ' __canvas_loadFontBytes gives.
      FAIL error(77050022, "font collection holds no faces: " & path)
    END IF
    dir = collections::getOr(faces, 0, 0)
  END IF
  RETURN __canvas_loadFontBytes(path, bytes, dir)
END FUNC

FUNC __canvas_loadFontBytes(path AS String, bytes AS List OF Byte, dir AS Integer) AS canvas::Font
  LET face AS List OF Byte = __canvas_extractFace(path, bytes, dir)
  IF NOT __canvas_isTrueType(face) THEN
    ' 77050022 is errorCode.ErrBadFontFile. The literal rather than the name because
    ' the injected builtin source does not IMPORT errorCode -- every other builtin
    ' body spells its codes the same way (crypto's helper_aes256_gcm_seal, and so on).
    FAIL error(77050022, "not a TrueType font: " & path)
  END IF
  ' `head.unitsPerEm` divides into every scale the renderer computes, and the format
  ' allows 16..16384 for it; FreeType refuses a file outside that range, and so does
  ' this (bug-509, DEC-53). At 1, a 300-unit glyph at size 100 is 30,000 px a side --
  ' one letter cost 62 s and 7.6 GB. A file with no `head` is left alone: it has no
  ' scale to poison, and its text measures and draws as nothing, as it always has.
  LET head AS Integer = __canvas_fontTable(face, "head")
  IF head >= 0 THEN
    LET upem AS Integer = __canvas_beU16(face, head + 18)
    IF upem < 16 OR upem > 16384 THEN
      FAIL error(77050022, "font unitsPerEm is outside 16..16384: " & path)
    END IF
  END IF
  RETURN canvas::fontFromBytes(face)
END FUNC"#;

/// `canvas::loadFont(path, face)` — load one named face of a file (plan-147-A).
///
/// A collection holds several faces, and the plain `loadFont(path)` takes the first; this
/// overload finds the one whose `name` table carries `face`. The PostScript name
/// (nameID 6, `Helvetica-Bold`) is tried across every face before the full name
/// (nameID 4, `Helvetica Bold`): the PostScript name is the unambiguous one, and it is
/// what the system-font members pass, because CoreText reports no face *index* to pass
/// instead. A plain sfnt is its own single face, so the same rule checks that a `.ttf`
/// is the face the caller meant.
///
/// A name no face carries is `ErrNotFound` (77050004), not `ErrBadFontFile`: the file is
/// a perfectly good font, it is the face that is missing, and the fix is a different name.
#[rustfmt::skip]
const LOAD_FONT_NAMED: &str =
r#"FUNC __canvas_loadFontNamed(path AS String, face AS String) AS canvas::Font
  LET bytes AS List OF Byte = fs::readBytes(path)
  LET faces AS List OF Integer = __canvas_sfntFaces(bytes)
  FOR EACH nameId IN [6, 4]
    FOR EACH dir IN faces
      IF __canvas_faceName(bytes, dir, nameId) = face THEN
        RETURN __canvas_loadFontBytes(path, bytes, dir)
      END IF
    NEXT
  NEXT
  FAIL error(77050004, "no face named " & face & " in " & path)
END FUNC"#;

/// TrueType Collections (plan-147-A): which faces a file holds, and one face lifted out
/// as a standalone sfnt.
///
/// A `ttcf` file is a header, a count, and one offset per face; each offset names an
/// ordinary table directory inside the same file, whose table offsets count from the
/// start of the *file*. Every reader downstream finds tables through
/// `__canvas_fontTable`, which reads the directory at byte 0 — so rather than thread a
/// face offset through a dozen helpers, the Font record and the graphics thread's font
/// table, the chosen face is copied out into a file of its own at load time. Nothing
/// after this point knows collections exist.
///
/// A plain sfnt is its own single face at directory `0`, and `__canvas_extractFace`
/// hands its bytes back untouched: an ordinary `.ttf` loads exactly as it always has.
///
/// The face count is capped at 4096 — real collections hold tens — so a hostile header
/// claiming four billion faces costs a comparison, not a loop (bug-509's principle).
/// A face whose directory or tables run past the end of the file is dropped or refused
/// rather than read past the end.
#[rustfmt::skip]
const COLLECTION: &str =
r#"FUNC __canvas_isCollection(bytes AS List OF Byte) AS Boolean
  IF len(bytes) < 12 THEN
    RETURN FALSE
  END IF
  RETURN __canvas_beU32(bytes, 0) = 1953784678
END FUNC

FUNC __canvas_sfntFaces(bytes AS List OF Byte) AS List OF Integer
  MUT faces AS List OF Integer = []
  IF NOT __canvas_isCollection(bytes) THEN
    faces = collections::append(faces, 0)
    RETURN faces
  END IF
  LET count AS Integer = __canvas_beU32(bytes, 8)
  IF count > 4096 THEN
    RETURN faces
  END IF
  MUT i AS Integer = 0
  WHILE i < count
    LET slot AS Integer = 12 + i * 4
    IF slot + 4 <= len(bytes) THEN
      LET dir AS Integer = __canvas_beU32(bytes, slot)
      IF dir >= 12 AND dir + 12 <= len(bytes) THEN
        faces = collections::append(faces, dir)
      END IF
    END IF
    i = i + 1
  END WHILE
  RETURN faces
END FUNC

FUNC __canvas_beBytes32(v AS Integer) AS List OF Byte
  RETURN [toByte((v / 16777216) MOD 256), toByte((v / 65536) MOD 256), toByte((v / 256) MOD 256), toByte(v MOD 256)]
END FUNC

FUNC __canvas_extractFace(path AS String, bytes AS List OF Byte, dir AS Integer) AS List OF Byte
  IF dir = 0 THEN
    RETURN bytes
  END IF
  LET numTables AS Integer = __canvas_beU16(bytes, dir + 4)
  IF dir + 12 + numTables * 16 > len(bytes) THEN
    FAIL error(77050022, "font collection face directory runs past the end of the file: " & path)
  END IF
  ' The header -- version, numTables and the search fields -- is the face's own; then
  ' one record per table with its offset rebased to the table's place in the new file.
  MUT out AS List OF Byte = collections::mid(bytes, dir, 12)
  MUT cursor AS Integer = 12 + numTables * 16
  MUT i AS Integer = 0
  WHILE i < numTables
    LET rec AS Integer = dir + 12 + i * 16
    LET at AS Integer = __canvas_beU32(bytes, rec + 8)
    LET size AS Integer = __canvas_beU32(bytes, rec + 12)
    IF at + size > len(bytes) THEN
      FAIL error(77050022, "font collection table runs past the end of the file: " & path)
    END IF
    out = collections::append(out, collections::mid(bytes, rec, 8))
    out = collections::append(out, __canvas_beBytes32(cursor))
    out = collections::append(out, __canvas_beBytes32(size))
    cursor = cursor + ((size + 3) / 4) * 4
    i = i + 1
  END WHILE
  ' The tables, each padded to four bytes as the format requires.
  i = 0
  WHILE i < numTables
    LET rec AS Integer = dir + 12 + i * 16
    LET at AS Integer = __canvas_beU32(bytes, rec + 8)
    LET size AS Integer = __canvas_beU32(bytes, rec + 12)
    out = collections::append(out, collections::mid(bytes, at, size))
    MUT pad AS Integer = ((size + 3) / 4) * 4 - size
    WHILE pad > 0
      out = collections::append(out, toByte(0))
      pad = pad - 1
    END WHILE
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

/// The sfnt version check, as its own helper so the accepted set is one readable list.
///
/// `0x00010000` is TrueType outlines and `true` is the Apple spelling of the same
/// thing. Everything else is refused *by name* rather than by falling through: `OTTO`
/// is CFF outlines (a different curve type and a different rasteriser), and `wOFF` /
/// `wOF2` are compressed web wrappers. Each is a real file a program might hand us,
/// and each deserves the same answer for a different reason.
///
/// A `ttcf` collection never reaches this check as a container: the loader lifts one
/// face out first (`COLLECTION`), and it is that face's version the check reads — so a
/// collection of CFF faces is refused here exactly as an `OTTO` file is.
#[rustfmt::skip]
const IS_TRUETYPE: &str =
r#"FUNC __canvas_isTrueType(bytes AS List OF Byte) AS Boolean
  IF len(bytes) < 12 THEN
    RETURN FALSE
  END IF
  LET b0 AS Integer = toInt(collections::getOr(bytes, 0, toByte(0)))
  LET b1 AS Integer = toInt(collections::getOr(bytes, 1, toByte(0)))
  LET b2 AS Integer = toInt(collections::getOr(bytes, 2, toByte(0)))
  LET b3 AS Integer = toInt(collections::getOr(bytes, 3, toByte(0)))
  IF b0 = 0 AND b1 = 1 AND b2 = 0 AND b3 = 0 THEN
    RETURN TRUE
  END IF
  IF b0 = 116 AND b1 = 114 AND b2 = 117 AND b3 = 101 THEN
    RETURN TRUE
  END IF
  RETURN FALSE
END FUNC"#;

/// `canvas::fontFromBytes(bytes) AS Font` — stamp the resource record.
///
/// The image twin of this is inside `createImage`; here it is a member of its own
/// because the *reading* half is MFBASIC and only the record stamping needs an
/// emitter. It is `internal_only`, so it does not render in `mfb man`: taking font
/// bytes from somewhere other than a file is a real thing to want, but it is not a
/// promise this letter is ready to make.
pub(crate) fn lower_font_from_bytes(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let bytes_in = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the font bytes"))?
        .location
        .clone();

    let alloc_ok = builder.label("canvas_font_alloc_ok");
    let done = builder.label("canvas_font_done");

    let bytes_slot = builder.allocate_stack_object("canvas_font_bytes", 8);
    builder.emit(abi::store_u64(&bytes_in, abi::stack_pointer(), bytes_slot));

    // Copy the file into runtime-owned storage. `copy_flat_block` on a collection is
    // shrink-to-fit, so the resource carries no caller headroom — which matters more
    // here than for pixels: a font file is measured in hundreds of kilobytes.
    let byte_list = ParameterType::list_of(ParameterType::Byte);
    let source = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), bytes_slot));
    let owned = builder.copy_flat_block(&byte_list, &source)?;
    let owned_slot = builder.allocate_stack_object("canvas_font_owned", 8);
    builder.emit(abi::store_u64(&owned, abi::stack_pointer(), owned_slot));

    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        RESOURCE_RECORD_SIZE,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&alloc_ok));

    let record = builder.temporary_vreg();
    builder.emit(abi::move_register(&record, abi::mfb_return(1)));
    let scratch = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&scratch, "Integer", RESOURCE_TAG_FONT));
    builder.emit(abi::store_u64(&scratch, &record, RESOURCE_OFFSET_TAG));
    // `handle@8` is the record's own address, exactly as `createImage` does it: unique
    // and non-zero for the resource's lifetime, so a `FontRef` is a real identity from
    // the start rather than a placeholder the backend has to replace.
    builder.emit(abi::store_u64(&record, &record, RESOURCE_OFFSET_HANDLE));
    builder.emit(abi::store_u64(abi::ZERO, &record, RESOURCE_OFFSET_CLOSED));
    builder.emit(abi::store_u64(abi::ZERO, &record, RESOURCE_OFFSET_STATE));
    let value = builder.temporary_vreg();
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), owned_slot));
    builder.emit(abi::store_u64(&value, &record, FONT_BYTES));
    // Publish it where the graphics thread can find it. The record is the worker's;
    // the renderer only ever has the integer `canvas::fontHandle` reads out of the font.
    super::gen_font_table::emit_register_font(
        builder,
        &Operand::from(record.to_string()),
        &Operand::from(value.to_string()),
    );

    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &record));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));

    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.fontFromBytes".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fontFromBytes",
        intro: "Stamp a `Font` resource around already-read font bytes.",
        desc: "The record-stamping half of `canvas::loadFont`, split out because \
               reading and validating the file is MFBASIC and only this needs an \
               emitter. Internal: it performs no validation of its own.",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "bytes",
                desc: "The whole font file.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::Byte),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::FONT_TYPE_ID),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_font_from_bytes),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "loadFont",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The font file to read.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::FONT_TYPE_ID),
            errors: vec!["ErrBadFontFile", "ErrOutOfMemory"],
            body: Body::mfb(LOAD_FONT, "__canvas_loadFont"),
        }, Implementation {
            params: vec![
                Parameter {
                    name: "path",
                    desc: "The font file to read.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "face",
                    desc: "The face to load: its PostScript name (`Helvetica-Bold`) \
                           or its full name (`Helvetica Bold`).",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::FONT_TYPE_ID),
            errors: vec!["ErrBadFontFile", "ErrNotFound", "ErrOutOfMemory"],
            body: Body::mfb(LOAD_FONT_NAMED, "__canvas_loadFontNamed"),
        }],
    });
    pkg.add_helper(RegistryHelper::always("canvas_isTrueType", IS_TRUETYPE));
    pkg.add_helper(RegistryHelper::always("canvas_collection", COLLECTION));
}
