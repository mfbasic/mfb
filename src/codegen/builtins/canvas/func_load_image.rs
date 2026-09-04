//! `canvas::loadImage` — decode an image file and hand the pixels to `createImage`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decode an image file and hold it as an `Image` resource."#;

const DESC: &str = r#"`loadImage` reads the file at `path`, decodes it to RGBA8, and returns the same
`Image` resource `canvas::createImage` produces — bound with `RES`, closing by itself
when it leaves scope, or sooner with `canvas::destroyImage`. A `canvas::Picture` item names
it through a `canvas::ImageRef`, never directly.

**PNG.** All five colour types (greyscale, truecolour, palette, greyscale+alpha,
truecolour+alpha), bit depths 1 through 16, `tRNS` transparency, and Adam7
interlacing. A file that is not a PNG, or a PNG whose chunks, filters or compressed
data are malformed, raises `ErrBadImageFile` — which is a different mistake from
`ErrNotFound`, a path that does not exist, and needs a different fix.

**Limits.** An image may be at most 16384 pixels a side and 16,777,216 pixels in
all, and its compressed data must fit the image its header declares. A file past
either — a header the file cannot fill, or a stream that inflates to more than the
image needs — is refused with `ErrBadImageFile` before any pixel is decoded.

Sixteen-bit samples are reduced to eight by taking the high byte, because the
destination is RGBA8. Colour management is not applied: `gAMA` and `iCCP` are read
past, and the pixels arrive as the file stores them.

A program that already has pixels — generated, or decoded by itself — wants
`canvas::createImage` instead; `loadImage` is that call with a decoder in front of it.

Requires `app::Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  RES logo AS canvas::Image = canvas::loadImage("logo.png")
  LET size AS canvas::Size = canvas::getSize(logo)
  LET art AS canvas::DrawItem = canvas::Picture[x := 20.0, y := 20.0, w := toFloat(size.width), h := toFloat(size.height), image := canvas::imageRef(logo), paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([art])
END SUB
```"#;

/// `canvas::loadImage(path)` — read, decode, and stamp.
///
/// MFBASIC for the same reason `loadFont` is: every step is a call it can already make.
/// The decoder is `helper_png.rs` on top of `helper_inflate.rs`, and the resource is
/// `canvas::createImage`, which already owns the record, the CPU shadow and the pixel
/// count contract. Nothing here needs an emitter, so nothing here has one.
///
/// The size is read from the header separately from the pixels. It could have been
/// packed in front of them, and that is exactly what the first draft did — a byte list
/// is the only thing the decoder can return — but packing two dimensions into bytes caps
/// the image at whatever the packing allows, for no reason but the container.
#[rustfmt::skip]
const LOAD_IMAGE: &str =
r#"FUNC __canvas_loadImage(path AS String) AS canvas::Image
  LET bytes AS List OF Byte = fs::readBytes(path)
  LET size AS List OF Integer = __canvas_pngSize(bytes)
  IF len(size) < 2 THEN
    ' 77050023 is errorCode.ErrBadImageFile. The literal rather than the name because
    ' the injected builtin source does not IMPORT errorCode.
    FAIL error(77050023, "not an image this build can decode: " & path)
  END IF
  LET pixels AS List OF Byte = __canvas_pngDecode(bytes)
  IF len(pixels) = 0 THEN
    FAIL error(77050023, "image file is malformed: " & path)
  END IF
  RETURN canvas::createImage(collections::getOr(size, 0, 0), collections::getOr(size, 1, 0), pixels)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "loadImage",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The image file to read.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::IMAGE_TYPE_ID),
            errors: vec![
                "ErrBadImageFile",
                "ErrBadPixelCount",
                "ErrOutOfMemory",
                "ErrWrongMode",
            ],
            body: Body::mfb(LOAD_IMAGE, "__canvas_loadImage"),
        }],
    });
}
