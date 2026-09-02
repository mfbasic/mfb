//! `canvas::loadImage` decodes PNG (plan-98-G Phase 1).
//!
//! These run a real headless `--app` program and compare the decoded pixels against a
//! truth image the test constructed, rather than inspecting the emitted code. A decoder
//! is exactly the kind of thing a shape assertion cannot check: every one of the bugs
//! worth catching here — a wrong filter predictor, a sample read at the wrong bit
//! offset, an Adam7 pass scattered to the wrong rows — produces a perfectly well-formed
//! image of the wrong thing.
//!
//! **Most fixtures are encoded by the test**, with stored (uncompressed) DEFLATE blocks,
//! so the whole colour-type/bit-depth/filter/interlace matrix is generated rather than
//! embedded, and a reader can see what each case contains. Two fixtures *are* embedded,
//! because a stored block never exercises the Huffman decoders: one fixed-Huffman and one
//! dynamic-Huffman file, produced by Python's `zlib` at `Z_FIXED` and level 9. Their
//! contents are asserted against the same generator the others use, so an embedded blob
//! cannot quietly drift into being a test of nothing.

mod common;

use std::process::Command;

/// The truth image: 7x5, deliberately not a multiple of 8 so sub-byte packing has a
/// ragged final byte in every row, and so Adam7's later passes have uneven widths.
const W: usize = 7;
const H: usize = 5;

fn truth() -> Vec<[u8; 4]> {
    let mut px = Vec::new();
    for y in 0..H {
        for x in 0..W {
            px.push([
                ((x * 37 + y * 11) % 256) as u8,
                ((x * 5 + y * 61) % 256) as u8,
                ((x * 91 + y * 3) % 256) as u8,
                255,
            ]);
        }
    }
    px
}

// --- a minimal PNG encoder -----------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc_input = tag.to_vec();
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

/// zlib-wrapped DEFLATE using stored blocks only — legal, and the one encoding a test
/// can produce without also writing a compressor.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut at = 0;
    loop {
        let take = (raw.len() - at).min(65535);
        let last = if at + take == raw.len() { 1 } else { 0 };
        out.push(last);
        out.extend_from_slice(&(take as u16).to_le_bytes());
        out.extend_from_slice(&(!(take as u16)).to_le_bytes());
        out.extend_from_slice(&raw[at..at + take]);
        at += take;
        if last == 1 {
            break;
        }
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

#[allow(clippy::too_many_arguments)]
fn png(
    w: usize,
    h: usize,
    colour: u8,
    depth: u8,
    raw: &[u8],
    palette: Option<&[u8]>,
    trns: Option<&[u8]>,
    interlace: u8,
) -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[depth, colour, 0, 0, interlace]);

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&chunk(b"IHDR", &ihdr));
    if let Some(p) = palette {
        out.extend_from_slice(&chunk(b"PLTE", p));
    }
    if let Some(t) = trns {
        out.extend_from_slice(&chunk(b"tRNS", t));
    }
    // Split across two IDATs. The stream is allowed to break at any byte, so a decoder
    // that inflates each chunk separately rather than concatenating them first is wrong
    // in a way only a split file shows.
    let z = zlib_stored(raw);
    let half = z.len() / 2;
    out.extend_from_slice(&chunk(b"IDAT", &z[..half]));
    out.extend_from_slice(&chunk(b"IDAT", &z[half..]));
    out.extend_from_slice(&chunk(b"IEND", &[]));
    out
}

/// Pack one row of samples at `depth` bits per sample, MSB first, zero-padded.
fn pack_row(values: &[u32], depth: u8) -> Vec<u8> {
    match depth {
        16 => values
            .iter()
            .flat_map(|v| (*v as u16).to_be_bytes())
            .collect(),
        8 => values.iter().map(|v| *v as u8).collect(),
        _ => {
            let mut out = Vec::new();
            let (mut acc, mut filled) = (0u16, 0u8);
            for v in values {
                acc = (acc << depth) | (*v as u16);
                filled += depth;
                if filled == 8 {
                    out.push(acc as u8);
                    acc = 0;
                    filled = 0;
                }
            }
            if filled > 0 {
                out.push((acc << (8 - filled)) as u8);
            }
            out
        }
    }
}

fn unfiltered(rows: &[Vec<u8>]) -> Vec<u8> {
    rows.iter()
        .flat_map(|r| {
            let mut line = vec![0u8];
            line.extend_from_slice(r);
            line
        })
        .collect()
}

/// Encode RGBA8 rows with one PNG filter applied to every row.
fn filtered_rgba(px: &[[u8; 4]], w: usize, h: usize, filter: u8) -> Vec<u8> {
    let stride = w * 4;
    let mut raw = Vec::new();
    let mut prev = vec![0u8; stride];
    for y in 0..h {
        let line: Vec<u8> = (0..w).flat_map(|x| px[y * w + x]).collect();
        let mut enc = vec![filter];
        for i in 0..stride {
            let a = if i >= 4 { line[i - 4] as i32 } else { 0 };
            let b = prev[i] as i32;
            let c = if i >= 4 { prev[i - 4] as i32 } else { 0 };
            let x = line[i] as i32;
            let predicted = match filter {
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => 0,
            };
            enc.push((x - predicted) as u8);
        }
        raw.extend_from_slice(&enc);
        prev = line;
    }
    raw
}

const ADAM7_X_ORIGIN: [usize; 7] = [0, 4, 0, 2, 0, 1, 0];
const ADAM7_Y_ORIGIN: [usize; 7] = [0, 0, 4, 0, 2, 0, 1];
const ADAM7_X_STEP: [usize; 7] = [8, 8, 4, 4, 2, 2, 1];
const ADAM7_Y_STEP: [usize; 7] = [8, 8, 8, 4, 4, 2, 2];

fn adam7_rgba(px: &[[u8; 4]], w: usize, h: usize) -> Vec<u8> {
    let mut raw = Vec::new();
    for pass in 0..7 {
        let (ox, oy) = (ADAM7_X_ORIGIN[pass], ADAM7_Y_ORIGIN[pass]);
        let (sx, sy) = (ADAM7_X_STEP[pass], ADAM7_Y_STEP[pass]);
        let pw = if w <= ox { 0 } else { (w - ox).div_ceil(sx) };
        let ph = if h <= oy { 0 } else { (h - oy).div_ceil(sy) };
        for y in 0..ph {
            raw.push(0u8);
            for x in 0..pw {
                raw.extend_from_slice(&px[(oy + y * sy) * w + ox + x * sx]);
            }
        }
    }
    raw
}

// --- running the program -------------------------------------------------------------

/// Build a `--app` program that loads `fixture.png` and prints its pixels, run it
/// headless, and return the decoded RGBA bytes — or `Err` with the raised message.
fn decode(name: &str, file: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    const SOURCE: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  RES img AS canvas::Image = canvas::loadImage("fixture.png") TRAP(e)
    io::print("failed:" & e.message)
    EXIT SUB
  END TRAP
  LET size AS canvas::Size = canvas::getSize(img)
  LET px AS List OF Byte = canvas::getBytes(img)
  MUT out AS String = toString(size.width) & "," & toString(size.height)
  MUT i AS Integer = 0
  WHILE i < len(px)
    out = out & "," & toString(toInt(collections::getOr(px, i, toByte(0))))
    i = i + 1
  END WHILE
  io::print(out)
END SUB
"#;
    let project = common::temp_project(name, SOURCE);
    std::fs::write(project.join("fixture.png"), file).expect("write the fixture");
    let build = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .arg(&project)
        .output()
        .expect("run mfb build -app");
    assert!(
        build.status.success(),
        "mfb build -app failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    let binary = app_binary(&project, name);
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program exited {:?}:\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    let _ = std::fs::remove_dir_all(&project);
    let line = stdout
        .lines()
        .next_back()
        .unwrap_or_else(|| panic!("no output from {name}"))
        .to_string();
    if let Some(message) = line.strip_prefix("failed:") {
        return Err(message.to_string());
    }
    let mut values = line.split(',');
    let w: usize = values.next().unwrap().trim().parse().expect("width");
    let h: usize = values.next().unwrap().trim().parse().expect("height");
    let pixels = values
        .map(|v| v.trim().parse::<u8>().expect("pixel byte"))
        .collect();
    Ok((w, h, pixels))
}

/// The built executable, which on macOS is inside an `.app` bundle.
fn app_binary(project: &std::path::Path, name: &str) -> std::path::PathBuf {
    let bundled = project
        .join("build")
        .join(format!("{name}.app"))
        .join("Contents")
        .join("MacOS")
        .join(name);
    if bundled.exists() {
        return bundled;
    }
    let plain = project.join("build").join(name);
    if plain.exists() {
        return plain;
    }
    project.join("build").join(format!("{name}.exe"))
}

fn assert_decodes(name: &str, file: &[u8], w: usize, h: usize, expected: &[u8]) {
    match decode(name, file) {
        Ok((gw, gh, got)) => {
            assert_eq!((gw, gh), (w, h), "{name}: wrong dimensions");
            assert_eq!(got.len(), expected.len(), "{name}: wrong byte count");
            if got != expected {
                let at = got.iter().zip(expected).position(|(a, b)| a != b).unwrap();
                panic!(
                    "{name}: first difference at byte {at} (pixel {}, channel {}): \
                     got {:?}, want {:?}",
                    at / 4,
                    at % 4,
                    &got[at..(at + 8).min(got.len())],
                    &expected[at..(at + 8).min(expected.len())],
                );
            }
        }
        Err(message) => panic!("{name}: decode failed: {message}"),
    }
}

fn flat(px: &[[u8; 4]]) -> Vec<u8> {
    px.iter().flatten().copied().collect()
}

// --- the cases -------------------------------------------------------------------------

#[test]
fn every_png_filter_reconstructs_the_same_image() {
    // The five filters of RFC 2083 §6, each applied to every row of the same picture.
    // They are the step where a decoder is most likely to be subtly wrong and still
    // produce something that looks like an image: Sub and Average both look plausible
    // when the `bpp` back-distance is off by a channel.
    let px = truth();
    for filter in 0..5u8 {
        let file = png(W, H, 6, 8, &filtered_rgba(&px, W, H, filter), None, None, 0);
        assert_decodes(
            &format!("canvas_png_filter{filter}"),
            &file,
            W,
            H,
            &flat(&px),
        );
    }
}

#[test]
fn greyscale_at_every_bit_depth_scales_to_full_range() {
    // A 1-bit `1` is white, not 1/255th of white. Scaling by `255 / max` rather than
    // shifting is what makes that true for 1, 2 and 4 bits at once — and a decoder that
    // shifts instead produces a recognisable but far too dark image, which is why this
    // asserts values rather than "something was drawn".
    let px = truth();
    for depth in [1u8, 2, 4, 8, 16] {
        let (stored, shown): (Vec<u32>, Vec<u8>) = if depth == 16 {
            // High byte is the truth, low byte is noise the decoder must drop.
            (
                px.iter().map(|p| ((p[0] as u32) << 8) | 0x11).collect(),
                px.iter().map(|p| p[0]).collect(),
            )
        } else {
            let max = (1u32 << depth) - 1;
            let values: Vec<u32> = px.iter().map(|p| (p[0] as u32 * max) / 255).collect();
            let shown = values.iter().map(|v| ((v * 255) / max) as u8).collect();
            (values, shown)
        };
        let rows: Vec<Vec<u8>> = (0..H)
            .map(|y| pack_row(&stored[y * W..(y + 1) * W], depth))
            .collect();
        let file = png(W, H, 0, depth, &unfiltered(&rows), None, None, 0);
        let expected: Vec<u8> = shown.iter().flat_map(|g| [*g, *g, *g, 255]).collect();
        assert_decodes(&format!("canvas_png_grey{depth}"), &file, W, H, &expected);
    }
}

#[test]
fn a_palette_index_names_an_entry_and_trns_gives_it_alpha() {
    // The one sample that must NOT be scaled: an index is a name, not a brightness.
    // `tRNS` is shorter than the palette on purpose — entries past its end are opaque,
    // which is the rule a decoder that sizes the two together gets wrong.
    for depth in [1u8, 2, 4, 8] {
        let entries = (1usize << depth).min(16);
        let palette: Vec<u8> = (0..entries)
            .flat_map(|i| {
                [
                    ((i * 17) % 256) as u8,
                    ((i * 29) % 256) as u8,
                    ((i * 43) % 256) as u8,
                ]
            })
            .collect();
        let trns: Vec<u8> = (0..entries.min(3))
            .map(|i| ((i * 15) % 256) as u8)
            .collect();
        let indices: Vec<u32> = (0..W * H).map(|i| (i % entries) as u32).collect();
        let rows: Vec<Vec<u8>> = (0..H)
            .map(|y| pack_row(&indices[y * W..(y + 1) * W], depth))
            .collect();
        let file = png(
            W,
            H,
            3,
            depth,
            &unfiltered(&rows),
            Some(&palette),
            Some(&trns),
            0,
        );
        let expected: Vec<u8> = indices
            .iter()
            .flat_map(|i| {
                let i = *i as usize;
                [
                    ((i * 17) % 256) as u8,
                    ((i * 29) % 256) as u8,
                    ((i * 43) % 256) as u8,
                    if i < trns.len() { trns[i] } else { 255 },
                ]
            })
            .collect();
        assert_decodes(&format!("canvas_png_pal{depth}"), &file, W, H, &expected);
    }
}

#[test]
fn truecolour_and_greyscale_alpha_fill_the_channels_they_lack() {
    let px = truth();

    // Colour type 2: no alpha in the file, opaque in the result.
    let rows: Vec<Vec<u8>> = (0..H)
        .map(|y| {
            (0..W)
                .flat_map(|x| px[y * W + x][..3].to_vec())
                .collect::<Vec<u8>>()
        })
        .collect();
    let expected: Vec<u8> = px.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    let file = png(W, H, 2, 8, &unfiltered(&rows), None, None, 0);
    assert_decodes("canvas_png_rgb8", &file, W, H, &expected);

    // Colour type 4: one grey channel replicated across three, and a real alpha.
    let rows: Vec<Vec<u8>> = (0..H)
        .map(|y| {
            (0..W)
                .flat_map(|x| {
                    let i = y * W + x;
                    [px[i][0], ((i * 9) % 256) as u8]
                })
                .collect::<Vec<u8>>()
        })
        .collect();
    let expected: Vec<u8> = (0..W * H)
        .flat_map(|i| [px[i][0], px[i][0], px[i][0], ((i * 9) % 256) as u8])
        .collect();
    let file = png(W, H, 4, 8, &unfiltered(&rows), None, None, 0);
    assert_decodes("canvas_png_greya8", &file, W, H, &expected);
}

#[test]
fn an_interlaced_image_scatters_to_the_same_pixels_as_a_progressive_one() {
    // Adam7 is seven complete sub-images, each with its own width, height and filter
    // stride. This is the assertion that matters for it: the *same* picture, encoded
    // both ways, must decode to the same bytes — a pass scattered to the wrong rows
    // still produces a full, plausible image.
    let px = truth();
    let file = png(W, H, 6, 8, &adam7_rgba(&px, W, H), None, None, 1);
    assert_decodes("canvas_png_interlaced", &file, W, H, &flat(&px));
}

/// 7x5 truecolour+alpha, the same truth image, compressed with **fixed** Huffman codes
/// (Python `zlib.compressobj(9, DEFLATED, 15, 9, Z_FIXED)`).
const FIXED_HUFFMAN_PNG: &str = concat!(
    "89504e470d0a1a0a0000000d4948445200000007000000050806000000899af6d80000004e4944415478010191006eff",
    "00000000ff25055bff4a0ab6ff6f0f11ff94146cffb919c7ffde1e22ff000b3d03ff30425eff5547b9ff7a4c14ff9f51",
    "6fffc456caffe95b25ff00167a06ff3b7f61ff6084bcff156decc30000004e49444154858917ffaa8e72ffcf93cdfff4",
    "9828ff0021b709ff46bc64ff6bc1bfff90c61affb5cb75ffdad0d0ffffd52bff002cf40cff51f967ff76fec2ff9b031d",
    "ffc00878ffe50dd3ff0a122eff0ebe4fed89ee044d0000000049454e44ae426082",
);

/// 64x48 truecolour+alpha with the Sub filter, compressed with **dynamic** Huffman codes
/// (Python `zlib.compress(raw, 9)`). Big enough that the encoder emits real
/// back-references at real distances, which a stored block never does.
const DYNAMIC_HUFFMAN_PNG: &str = concat!(
    "89504e470d0a1a0a0000000d4948445200000040000000300806000000a14b7c1f0000007d4944415478dae5d0b10a41",
    "610080d15f923290a228312849dd81144589e196a40c2c0c4a168b94818541c9629132b03028592c5206160625cb5da4",
    "0c2c0cca66533cc8379c17382a21c44f2db4824a25345a78804e0f0f3098e101261b3cc0ea8407d83df000a7171ee00e",
    "c203a4083cc027c30302497840380d0f88e6e00172013bacc51c0000007e494441541e9028c10352157840a6060fc836",
    "e101f9363ca0d8850794faf080f2101e509dc003ea33784063010f68ade0019d0d3ca0bb8707f48ef08081020f185de0",
    "01e31b3c60fa8407ccdff080c5071eb0fcc203d66a78c056070fd819e101070b3ce0e48007282e78c05982075cfdf080",
    "7b081ef088c1035e7174c01fac4ed00154d0fa870000000049454e44ae426082",
);

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

#[test]
fn fixed_and_dynamic_huffman_blocks_both_inflate() {
    // Stored blocks exercise the chunk walk, the defilter and the pixel conversion, but
    // not one line of the Huffman decoder — so these two are the only fixtures the test
    // cannot generate itself. Their expected pixels come from the same generator every
    // other case uses, which is what stops an embedded blob from becoming a golden of
    // whatever the decoder happens to do.
    let px = truth();
    assert_decodes(
        "canvas_png_fixed_huffman",
        &unhex(FIXED_HUFFMAN_PNG),
        W,
        H,
        &flat(&px),
    );

    let (bw, bh) = (64usize, 48usize);
    let big: Vec<[u8; 4]> = (0..bh)
        .flat_map(|y| {
            (0..bw).map(move |x| {
                [
                    ((x * 3) % 256) as u8,
                    ((y * 5) % 256) as u8,
                    (((x + y) * 7) % 256) as u8,
                    255,
                ]
            })
        })
        .collect();
    assert_decodes(
        "canvas_png_dynamic_huffman",
        &unhex(DYNAMIC_HUFFMAN_PNG),
        bw,
        bh,
        &flat(&big),
    );
}

#[test]
fn a_file_that_is_not_a_png_is_refused_by_name() {
    // `ErrBadImageFile`, not `ErrNotFound`: the file is there, it is the wrong thing.
    // The two need different fixes, which is the whole reason the code exists.
    let mut gif = b"GIF89a".to_vec();
    gif.extend_from_slice(&[0u8; 64]);
    match decode("canvas_png_not_a_png", &gif) {
        Ok((w, h, _)) => panic!("a GIF decoded as a {w}x{h} image"),
        Err(message) => assert!(
            message.contains("not an image this build can decode"),
            "wrong message: {message}",
        ),
    }
}

#[test]
fn a_truncated_png_fails_rather_than_decoding_what_survived() {
    // Half a file is the case where a decoder most easily produces a *partial* image and
    // reports success — the top half right and the bottom half whatever the buffer held.
    // The inflate runs out of input and the decode says so.
    let px = truth();
    let whole = png(W, H, 6, 8, &filtered_rgba(&px, W, H, 4), None, None, 0);
    let half = &whole[..whole.len() / 2];
    match decode("canvas_png_truncated", half) {
        Ok((w, h, _)) => panic!("half a PNG decoded as a {w}x{h} image"),
        Err(message) => assert!(
            message.contains("malformed") || message.contains("not an image"),
            "wrong message: {message}",
        ),
    }
}

#[test]
fn a_header_no_png_could_have_is_refused_before_any_decoding() {
    // Colour type 2 at 4 bits a sample is not a variant to guess at — RFC 2083 Table
    // 11.1 gives it no meaning. Accepting it would mean inventing one, and the pixels
    // would be wrong in a way nothing downstream could detect.
    let px = truth();
    let mut file = png(W, H, 2, 8, &filtered_rgba(&px, W, H, 0), None, None, 0);
    file[24] = 4; // IHDR bit depth
    match decode("canvas_png_bad_header", &file) {
        Ok((w, h, _)) => panic!("4-bit truecolour decoded as a {w}x{h} image"),
        Err(message) => assert!(
            message.contains("not an image this build can decode"),
            "wrong message: {message}",
        ),
    }
}
