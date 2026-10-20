//! plan-176-J: `SELF_UPDATE_TABLE`, one row per self-update-shaped overload. The row
//! types, package consts and `probe*` builders live in `census/mod.rs`.

use super::*;

/// Every registry function with a self-update-shaped overload
/// (`registry::self_update_shaped`), plus the `String` self-concat.
#[cfg(test)]
pub(crate) const SELF_UPDATE_TABLE: &[SelfUpdateRow] = &[
    // --- in place today (site S1) ---
    SelfUpdateRow {
        function: "collections::append",
        kind: SelfUpdate::Arm(&[ArmId::Append, ArmId::BulkAppend]),
        probes: &[
            probe(C, LI, "[1, 2, 3]", "collections::append(x, 4)"),
            probe(C, LI, "[1, 2, 3]", "collections::append(x, [4, 5])"),
        ],
    },
    SelfUpdateRow {
        function: "collections::set",
        kind: SelfUpdate::Arm(&[ArmId::Set]),
        probes: &[
            probe(C, LI, "[1, 2, 3]", "collections::set(x, 0, 9)"),
            probe(
                C,
                MSI,
                "Map OF String TO Integer { \"a\" := 1 }",
                "collections::set(x, \"b\", 2)",
            ),
        ],
    },
    SelfUpdateRow {
        function: "collections::add",
        kind: SelfUpdate::Arm(&[ArmId::SetAdd]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::add(x, 3)",
        )],
    },
    SelfUpdateRow {
        function: "collections::remove",
        kind: SelfUpdate::Arm(&[ArmId::SetRemove]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::remove(x, 1)",
        )],
    },
    SelfUpdateRow {
        function: "collections::removeKey",
        kind: SelfUpdate::Arm(&[ArmId::RemoveKey]),
        probes: &[probe(
            C,
            MSI,
            "Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }",
            "collections::removeKey(x, \"a\")",
        )],
    },
    SelfUpdateRow {
        function: "collections::prepend",
        kind: SelfUpdate::Arm(&[ArmId::Prepend]),
        probes: &[probe(C, LI, "[1, 2, 3]", "collections::prepend(x, 0)")],
    },
    SelfUpdateRow {
        function: "collections::removeAt",
        kind: SelfUpdate::Arm(&[ArmId::RemoveAt]),
        probes: &[probe(
            C,
            LI,
            "[1, 2, 3, 4, 5]",
            "collections::removeAt(x, 0)",
        )],
    },
    SelfUpdateRow {
        function: "collections::insert",
        kind: SelfUpdate::Arm(&[ArmId::Insert]),
        probes: &[probe(C, LI, "[1, 2, 3]", "collections::insert(x, 1, 7)")],
    },
    SelfUpdateRow {
        function: "collections::repeat",
        kind: SelfUpdate::Arm(&[ArmId::Repeat]),
        probes: &[probe(C, LI, "[1, 2, 3]", "collections::repeat(x, 7, 2, 2)")],
    },
    SelfUpdateRow {
        function: "&",
        kind: SelfUpdate::Arm(&[ArmId::Concat]),
        probes: &[probe(&[], "String", "\"a\"", "x & \"b\"")],
    },
    // --- letter B: shrink / compact ---
    SelfUpdateRow {
        function: "collections::filter",
        kind: SelfUpdate::Arm(&[ArmId::Filter]),
        probes: &[probe_with(
            C,
            IS_POSITIVE,
            LI,
            "[1, -2, 3]",
            "collections::filter(x, isPositive)",
        )],
    },
    SelfUpdateRow {
        function: "collections::take",
        kind: SelfUpdate::Arm(&[ArmId::Take]),
        probes: &[probe(C, LI, "[1, 2, 3, 4, 5]", "collections::take(x, 4)")],
    },
    SelfUpdateRow {
        function: "collections::drop",
        kind: SelfUpdate::Arm(&[ArmId::Drop]),
        probes: &[probe(C, LI, "[1, 2, 3, 4, 5]", "collections::drop(x, 1)")],
    },
    SelfUpdateRow {
        function: "collections::mid",
        kind: SelfUpdate::Arm(&[ArmId::Mid]),
        probes: &[probe(C, LI, "[1, 2, 3, 4, 5]", "collections::mid(x, 0, 4)")],
    },
    SelfUpdateRow {
        function: "collections::distinct",
        kind: SelfUpdate::Arm(&[ArmId::Distinct]),
        probes: &[probe(C, LI, "[1, 1, 2]", "collections::distinct(x)")],
    },
    // --- letter C: element rewrite / reorder ---
    SelfUpdateRow {
        function: "collections::replace",
        kind: SelfUpdate::Arm(&[ArmId::Replace]),
        probes: &[probe(C, LI, "[1, 2, 1]", "collections::replace(x, 1, 5)")],
    },
    SelfUpdateRow {
        function: "collections::transform",
        kind: SelfUpdate::Arm(&[ArmId::Transform]),
        probes: &[probe_with(
            C,
            NEGATED,
            LI,
            "[1, 2, 3]",
            "collections::transform(x, negated)",
        )],
    },
    SelfUpdateRow {
        function: "collections::sort",
        kind: SelfUpdate::Arm(&[ArmId::Sort]),
        probes: &[
            probe(C, LI, "[3, 1, 2]", "collections::sort(x)"),
            probe(
                C,
                "List OF String",
                "[\"b\", \"a\"]",
                "collections::sort(x)",
            ),
            probe(C, LF, FLOATS, "collections::sort(x)"),
            probe(
                C,
                "List OF Byte",
                "[toByte(3), toByte(1)]",
                "collections::sort(x)",
            ),
        ],
    },
    SelfUpdateRow {
        function: "collections::sortBy",
        kind: SelfUpdate::Arm(&[ArmId::SortBy]),
        probes: &[
            probe_with(
                C,
                NEGATED,
                LI,
                "[3, 1, 2]",
                "collections::sortBy(x, negated)",
            ),
            probe_with(
                C,
                KEYLEN,
                "List OF String",
                "[\"bb\", \"a\"]",
                "collections::sortBy(x, keyLen)",
            ),
            probe_with(C, KEYSTR, LI, "[3, 1, 2]", "collections::sortBy(x, keyStr)"),
        ],
    },
    SelfUpdateRow {
        function: "math::abs",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::abs(x)"),
            probe(M, LI, "[1, -2, 3]", "math::abs(x)"),
            probe(M, LX, FIXEDS, "math::abs(x)"),
        ],
    },
    SelfUpdateRow {
        function: "math::acos",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::acos(x)")],
    },
    SelfUpdateRow {
        function: "math::asin",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::asin(x)")],
    },
    SelfUpdateRow {
        function: "math::atan",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::atan(x)")],
    },
    SelfUpdateRow {
        function: "math::atan2",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::atan2(x, [1.0, 1.0, 1.0])")],
    },
    SelfUpdateRow {
        function: "math::clamp",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::clamp(x, 0.0, 0.25)"),
            probe(M, LI, "[1, -2, 3]", "math::clamp(x, -1, 2)"),
            probe(M, LX, FIXEDS, "math::clamp(x, 0.5F, 2.0F)"),
        ],
    },
    SelfUpdateRow {
        function: "math::cos",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::cos(x)")],
    },
    SelfUpdateRow {
        function: "math::exp",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::exp(x)")],
    },
    SelfUpdateRow {
        function: "math::log",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::log(x)"),
            probe(M, LX, FIXEDS, "math::log(x)"),
        ],
    },
    SelfUpdateRow {
        function: "math::log10",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::log10(x)"),
            probe(M, LX, FIXEDS, "math::log10(x)"),
        ],
    },
    SelfUpdateRow {
        function: "math::max",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::max(x, [0.5, 0.0, 0.5])"),
            probe(M, LI, "[1, -2, 3]", "math::max(x, [2, 2, 2])"),
            probe(M, LX, FIXEDS, "math::max(x, [1.0F, 1.0F, 1.0F])"),
        ],
    },
    SelfUpdateRow {
        function: "math::min",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::min(x, [0.5, 0.0, 0.5])"),
            probe(M, LI, "[1, -2, 3]", "math::min(x, [2, 2, 2])"),
            probe(M, LX, FIXEDS, "math::min(x, [1.0F, 1.0F, 1.0F])"),
        ],
    },
    SelfUpdateRow {
        function: "math::pow",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::pow(x, [1.0, 2.0, 1.0])")],
    },
    SelfUpdateRow {
        function: "math::sin",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::sin(x)")],
    },
    SelfUpdateRow {
        function: "math::sqrt",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::sqrt(x)"),
            probe(M, LX, FIXEDS, "math::sqrt(x)"),
        ],
    },
    SelfUpdateRow {
        function: "math::tan",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::tan(x)")],
    },
    // --- letter D: Set algebra and Map ---
    SelfUpdateRow {
        function: "collections::union",
        kind: SelfUpdate::Arm(&[ArmId::Union]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::union(x, Set OF Integer { 2, 3 })",
        )],
    },
    SelfUpdateRow {
        function: "collections::intersection",
        kind: SelfUpdate::Arm(&[ArmId::Intersection]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::intersection(x, Set OF Integer { 2, 3 })",
        )],
    },
    SelfUpdateRow {
        function: "collections::difference",
        kind: SelfUpdate::Arm(&[ArmId::Difference]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::difference(x, Set OF Integer { 2, 3 })",
        )],
    },
    SelfUpdateRow {
        function: "collections::symmetricDifference",
        kind: SelfUpdate::Arm(&[ArmId::SymmetricDifference]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::symmetricDifference(x, Set OF Integer { 2, 3 })",
        )],
    },
    SelfUpdateRow {
        function: "collections::merge",
        kind: SelfUpdate::Arm(&[ArmId::Merge]),
        probes: &[probe(
            C,
            MSI,
            "Map OF String TO Integer { \"a\" := 1 }",
            "collections::merge(x, Map OF String TO Integer { \"b\" := 2 }, TRUE)",
        )],
    },
    SelfUpdateRow {
        function: "collections::mapValues",
        kind: SelfUpdate::Arm(&[ArmId::MapValues]),
        probes: &[probe_with(
            C,
            NEGATED,
            MSI,
            "Map OF String TO Integer { \"a\" := 1 }",
            "collections::mapValues(x, negated)",
        )],
    },
    // --- letter E: exempt, proven copy-free ---
    SelfUpdateRow {
        function: "collections::reduce",
        kind: SelfUpdate::Exempt {
            reason: REDUCE_REASON,
            proof: REDUCE_PROOF,
        },
        probes: &[probe_with(
            C,
            PUSH,
            LI,
            "[1, 2, 3]",
            "collections::reduce(x, [0], push)",
        )],
    },
    SelfUpdateRow {
        function: "collections::reduceRight",
        kind: SelfUpdate::Exempt {
            reason: REDUCE_REASON,
            proof: REDUCE_PROOF,
        },
        probes: &[probe_with(
            C,
            PUSH,
            LI,
            "[1, 2, 3]",
            "collections::reduceRight(x, [0], push)",
        )],
    },
    SelfUpdateRow {
        function: "compress::deflate",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(Z, LB, BYTES, "compress::deflate(x, 6)")],
    },
    SelfUpdateRow {
        function: "compress::inflate",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(
            Z,
            LB,
            "compress::deflate(encoding::utf8Encode(\"hello\"), 6)",
            "compress::inflate(x, 1048576)",
        )],
    },
    SelfUpdateRow {
        function: "compress::gzipEncode",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(Z, LB, BYTES, "compress::gzipEncode(x, 6)")],
    },
    SelfUpdateRow {
        function: "compress::gzipDecode",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(
            Z,
            LB,
            "compress::gzipEncode(encoding::utf8Encode(\"hello\"), 6)",
            "compress::gzipDecode(x, 1048576, FALSE)",
        )],
    },
    SelfUpdateRow {
        function: "compress::zlibEncode",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(Z, LB, BYTES, "compress::zlibEncode(x, 6)")],
    },
    SelfUpdateRow {
        function: "compress::zlibDecode",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(
            Z,
            LB,
            "compress::zlibEncode(encoding::utf8Encode(\"hello\"), 6)",
            "compress::zlibDecode(x, 1048576, FALSE)",
        )],
    },
    SelfUpdateRow {
        function: "crypto::argon2id",
        kind: SelfUpdate::Exempt {
            reason: ARGON_REASON,
            proof: ARGON_PROOF,
        },
        probes: &[probe(
            K,
            LB,
            BYTES,
            "crypto::argon2id(x, encoding::utf8Encode(\"saltsaltsalt\"), 32, 1, 1, 32)",
        )],
    },
    SelfUpdateRow {
        function: "crypto::shake256",
        kind: SelfUpdate::Exempt {
            reason: SHAKE_REASON,
            proof: SHAKE_PROOF,
        },
        probes: &[probe(K, LB, BYTES, "crypto::shake256(x, 32)")],
    },
    // --- plan-146: the `String` self-updates ---
    SelfUpdateRow {
        function: "toString",
        kind: SelfUpdate::Arm(&[ArmId::StrIdentity]),
        probes: &[str_probe(&[], STR, "toString(x)")],
    },
    SelfUpdateRow {
        function: "strings::left",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::left(x, 3)")],
    },
    SelfUpdateRow {
        function: "strings::right",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::right(x, 3)")],
    },
    SelfUpdateRow {
        function: "strings::mid",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::mid(x, 1, 3)")],
    },
    SelfUpdateRow {
        function: "strings::stripPrefix",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::stripPrefix(x, \"a\")")],
    },
    SelfUpdateRow {
        function: "strings::stripSuffix",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::stripSuffix(x, \"f\")")],
    },
    SelfUpdateRow {
        function: "strings::trim",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::trim(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimStart",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::trimStart(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimEnd",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::trimEnd(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimChars",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::trimChars(x, \"a\")")],
    },
    SelfUpdateRow {
        function: "strings::graphemeAt",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(ST, STR, "strings::graphemeAt(x, 0)")],
    },
    SelfUpdateRow {
        function: "fs::pathBaseName",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(FS, PATH, "fs::pathBaseName(x)")],
    },
    SelfUpdateRow {
        function: "fs::pathDirName",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(FS, PATH, "fs::pathDirName(x)")],
    },
    SelfUpdateRow {
        function: "fs::pathExtension",
        kind: SelfUpdate::Arm(&[ArmId::StrWindow]),
        probes: &[str_probe(FS, PATH, "fs::pathExtension(x)")],
    },
    SelfUpdateRow {
        function: "strings::padLeft",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(ST, STR, "strings::padLeft(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padRight",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(ST, STR, "strings::padRight(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padLeftToWidth",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(ST, STR, "strings::padLeftToWidth(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padRightToWidth",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(ST, STR, "strings::padRightToWidth(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::repeat",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(ST, STR, "strings::repeat(x, 2)")],
    },
    SelfUpdateRow {
        function: "os::appResourcePath",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(OS, STR, "os::appResourcePath(x)")],
    },
    // plan-157-B: `GrowKind::HostPath` — the base is `f("")`, re-acquired per
    // statement (it follows `HOME`), then `x` grows by `base` + `/`.
    SelfUpdateRow {
        function: "os::appDataPath",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(OS, STR, "os::appDataPath(x)")],
    },
    SelfUpdateRow {
        function: "os::appCachePath",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(OS, STR, "os::appCachePath(x)")],
    },
    // plan-157-C: the same `GrowKind::HostPath` arm.
    SelfUpdateRow {
        function: "os::userHomePath",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(OS, STR, "os::userHomePath(x)")],
    },
    SelfUpdateRow {
        function: "os::userDocumentsPath",
        kind: SelfUpdate::Arm(&[ArmId::StrGrow]),
        probes: &[str_probe(OS, STR, "os::userDocumentsPath(x)")],
    },
    SelfUpdateRow {
        function: "strings::upper",
        kind: SelfUpdate::Arm(&[ArmId::StrRewrite]),
        probes: &[str_probe(ST, STR, "strings::upper(x)")],
    },
    SelfUpdateRow {
        function: "strings::lower",
        kind: SelfUpdate::Arm(&[ArmId::StrRewrite]),
        probes: &[str_probe(ST, STR, "strings::lower(x)")],
    },
    SelfUpdateRow {
        function: "strings::caseFold",
        kind: SelfUpdate::Arm(&[ArmId::StrRewrite]),
        probes: &[str_probe(ST, STR, "strings::caseFold(x)")],
    },
    SelfUpdateRow {
        function: "strings::normalizeNfc",
        kind: SelfUpdate::Arm(&[ArmId::StrRewrite]),
        probes: &[str_probe(ST, STR, "strings::normalizeNfc(x)")],
    },
    SelfUpdateRow {
        function: "strings::replace",
        kind: SelfUpdate::Arm(&[ArmId::StrRewrite]),
        probes: &[str_probe(ST, STR, "strings::replace(x, \"a\", \"b\")")],
    },
    SelfUpdateRow {
        function: "fs::pathNormalize",
        kind: SelfUpdate::Arm(&[ArmId::StrRewrite]),
        probes: &[str_probe(FS, PATH, "fs::pathNormalize(x)")],
    },
    SelfUpdateRow {
        function: "encoding::formUrlDecode",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__encoding_formUrlDecode` (`encoding/func_form_url_decode.rs`) delegates to `__encoding_percentDecodeBytes` (`encoding/helper_percent_decode_bytes.rs`), whose output starts at `MUT result AS List OF Byte = []` and which reads `text` only through `strings::toBytes` and `collections::get`: no binding of `text`, so no copy.",
        },
        probes: &[str_probe(EN, STR, "encoding::formUrlDecode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::formUrlEncode",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__encoding_formUrlEncode` (`encoding/func_form_url_encode.rs`) starts its output at `MUT out AS String = \"\"` and reads `text` once, through `LET data AS List OF Byte = strings::toBytes(text)` and a `FOR EACH` over `data`: no binding of `text`, so no copy.",
        },
        probes: &[str_probe(EN, STR, "encoding::formUrlEncode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::htmlEscape",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__encoding_htmlEscape` (`encoding/func_html_escape.rs`) starts its output at `MUT out AS String = \"\"` and walks `text` left to right with `strings::mid(text, i, 1)`, appending one reference or one grapheme per step: the only thing live besides `text` is `out`. plan-146-F rewrote it; it used to start at `MUT out AS String = text`, which copied the argument.",
        },
        probes: &[str_probe(EN, STR, "encoding::htmlEscape(x)")],
    },
    SelfUpdateRow {
        function: "encoding::htmlUnescape",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__encoding_htmlUnescape` (`encoding/func_html_unescape.rs`) starts its output at `MUT out AS String = \"\"` and walks `text` left to right with `strings::mid(text, i, 1)`: no binding of `text`, and no grapheme list held beside it (plan-146-F Correction F4).",
        },
        probes: &[str_probe(EN, STR, "encoding::htmlUnescape(x)")],
    },
    SelfUpdateRow {
        function: "encoding::percentDecode",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__encoding_percentDecode` (`encoding/func_percent_decode.rs`) delegates to `__encoding_percentDecodeBytes` (`encoding/helper_percent_decode_bytes.rs`), whose output starts at `MUT result AS List OF Byte = []` and which reads `text` only through `strings::toBytes` and `collections::get`: no binding of `text`, so no copy.",
        },
        probes: &[str_probe(EN, STR, "encoding::percentDecode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::percentEncode",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__encoding_percentEncode` (`encoding/func_percent_encode.rs`) starts its output at `MUT out AS String = \"\"` and reads `text` once, through `LET data AS List OF Byte = strings::toBytes(text)` and a `FOR EACH` over `data`: no binding of `text`, so no copy.",
        },
        probes: &[str_probe(EN, STR, "encoding::percentEncode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::punycodeDecode",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__encoding_punycodeDecode` (`encoding/func_punycode_decode.rs`) starts its output at `MUT out AS String = \"\"` and reads `asciiDomain` once, through `strings::split(asciiDomain, \".\")`, then appends per label: `asciiDomain` itself is never bound, and the label list `split` returns is the decoder's own working value, not a second name for the argument block.",
        },
        probes: &[str_probe(EN, STR, "encoding::punycodeDecode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::punycodeEncode",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__encoding_punycodeEncode` (`encoding/func_punycode_encode.rs`) starts its output at `MUT out AS String = \"\"` and reads `domain` once, through `strings::split(domain, \".\")`, then appends per label: `domain` itself is never bound, and the label list `split` returns is the encoder's own working value, not a second name for the argument block.",
        },
        probes: &[str_probe(EN, STR, "encoding::punycodeEncode(x)")],
    },
    SelfUpdateRow {
        function: "fs::canonicalPath",
        kind: SelfUpdate::Exempt {
            reason: NOT_DERIVED_REASON,
            proof: "`lower_fs_canonical_path_helper` (`builtins/fs/gen_canonical.rs`) hands `realpath` a pointer into the caller's block (`path + 8`, already NUL-terminated) and copies the result out of its own PATH_MAX buffer; the only walk of `path` is `emit_cstring_nul_scan` (`builtins/fs/gen_shared.rs`), which reads bytes and stores none. plan-146-F replaced the `len + 1` arena copy this used to marshal (finding F4).",
        },
        probes: &[str_probe(FS, PATH, "fs::canonicalPath(x)")],
    },
    SelfUpdateRow {
        function: "fs::readText",
        kind: SelfUpdate::Exempt {
            reason: NOT_DERIVED_REASON,
            proof: "`lower_fs_read_text_path_helper` (`builtins/fs/gen_atomic_write/read_path.rs`) hands `open` a pointer into the caller's block (`path + 8`, already NUL-terminated) and fills the result `String` from `read`; the only walk of `path` is `emit_cstring_nul_scan` (`builtins/fs/gen_shared.rs`), which reads bytes and stores none. plan-146-F replaced the `len + 1` arena copy this used to marshal (finding F4).",
        },
        probes: &[str_probe(FS, PATH, "fs::readText(x)")],
    },
    SelfUpdateRow {
        function: "io::input",
        kind: SelfUpdate::Exempt {
            reason: NOT_DERIVED_REASON,
            proof: "`lower_read_line_family` (`builtins/io/gen_read_line_family.rs`) writes the prompt to stdout straight out of the caller's block (`prompt + 8`, with the stored length) and builds the result from the stdin line buffer: the prompt is stored nowhere.",
        },
        probes: &[str_probe(&[], STR, "io::input(x)")],
    },
    SelfUpdateRow {
        function: "net::percentDecode",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__net_percentDecode` (`net/func_percent_decode.rs`) delegates to `__net_percentDecodeImpl` (`net/helper_percent_decode_impl.rs`), whose output starts at `MUT out AS List OF Byte = []` and which reads `s` only through `len` and a per-grapheme `strings::mid`: no binding of `s`, so no copy.",
        },
        probes: &[str_probe(NET, STR, "net::percentDecode(x)")],
    },
    SelfUpdateRow {
        function: "os::getEnv",
        kind: SelfUpdate::Exempt {
            reason: NOT_DERIVED_REASON,
            proof: "`lower_get_env` (`builtins/os/gen_env.rs`) hands `getenv` a pointer into the caller's block (`borrow_cstring`, `builtins/os/gen_shared.rs`) and builds the result from the host's C string (`build_string_from_cstr`): `name` is read by the host and stored nowhere. plan-146-F replaced the `len + 1` arena copy `marshal_cstring` used to make (finding F4).",
        },
        probes: &[str_probe(OS, STR, "os::getEnv(x)")],
    },
    SelfUpdateRow {
        function: "os::getEnvOr",
        kind: SelfUpdate::Exempt {
            reason: NOT_DERIVED_REASON,
            proof: "`lower_get_env` with `with_fallback` (`builtins/os/gen_env.rs`) hands `getenv` a pointer into the caller's block (`borrow_cstring`, `builtins/os/gen_shared.rs`); the not-found path copies the `fallback` argument, never `name`. plan-146-F replaced the `len + 1` arena copy `marshal_cstring` used to make of `name` (finding F4).",
        },
        probes: &[str_probe(OS, STR, "os::getEnvOr(x, \"x\")")],
    },
    SelfUpdateRow {
        function: "os::getConfigOr",
        kind: SelfUpdate::Exempt {
            reason: "The self-update form cannot be built: `key` must be a String literal, so no call ever reaches codegen with `x` as its first argument.",
            proof: "`ir::shape`'s `check_config_call_literals` rejects a non-literal `key` with `OS_CONFIG_ARGUMENT_NOT_LITERAL` (2-203-0141), so this row's probe, `os::getConfigOr(x, \"x\")`, does not build; every call that does build is replaced by a constant in `target/shared/config_fold.rs:fold_config_calls` before NIR exists (plan-167-B).",
        },
        probes: &[str_probe(OS, STR, "os::getConfigOr(x, \"x\")")],
    },
    SelfUpdateRow {
        function: "regex::replace",
        kind: SelfUpdate::Exempt {
            reason: CODEC_REASON,
            proof: "`Body::mfb` `__regex_replace` (`regex/func_replace.rs`) starts its output at `MUT out AS String = \"\"` and appends only the slices between matches (`strings::mid(value, cursor, …)`) and the expanded replacements; the engine sees `value` as the codepoint list `__regex_makeCtx` derives (`regex/helper_make_ctx.rs`), not as the block: no binding of `value`, so no copy.",
        },
        probes: &[str_probe(RE, STR, "regex::replace(x, \"a\", \"b\")")],
    },
    // --- plan-146-A Open Decision 1: `AttributedString`, deferred ---
    SelfUpdateRow {
        function: "astrings::addAttribute",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AS,
            ATTR,
            ATTR_INIT,
            "astrings::addAttribute(x, 0, 1, astrings::bold())",
        )],
    },
    SelfUpdateRow {
        function: "astrings::clearAttributes",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[
            probe(AS, ATTR, ATTR_INIT, "astrings::clearAttributes(x)"),
            probe(AS, ATTR, ATTR_INIT, "astrings::clearAttributes(x, 0, 1)"),
        ],
    },
    SelfUpdateRow {
        function: "astrings::removeAttribute",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AS,
            ATTR,
            ATTR_INIT,
            "astrings::removeAttribute(x, 0, 1, astrings::bold())",
        )],
    },
    SelfUpdateRow {
        function: "&@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AS,
            ATTR,
            ATTR_INIT,
            "x & astrings::fromString(\"b\")",
        )],
    },
    SelfUpdateRow {
        function: "strings::left@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::left(x, 3)")],
    },
    SelfUpdateRow {
        function: "strings::right@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::right(x, 3)")],
    },
    SelfUpdateRow {
        function: "strings::mid@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::mid(x, 1, 3)")],
    },
    SelfUpdateRow {
        function: "strings::trim@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::trim(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimStart@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::trimStart(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimEnd@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::trimEnd(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimChars@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::trimChars(x, \"a\")")],
    },
    SelfUpdateRow {
        function: "strings::stripPrefix@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AST,
            ATTR,
            ATTR_INIT,
            "strings::stripPrefix(x, \"a\")",
        )],
    },
    SelfUpdateRow {
        function: "strings::stripSuffix@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AST,
            ATTR,
            ATTR_INIT,
            "strings::stripSuffix(x, \"f\")",
        )],
    },
    SelfUpdateRow {
        function: "strings::padLeft@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::padLeft(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padLeftToWidth@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::padLeftToWidth(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padRight@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::padRight(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padRightToWidth@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AST,
            ATTR,
            ATTR_INIT,
            "strings::padRightToWidth(x, 8)",
        )],
    },
    SelfUpdateRow {
        function: "strings::repeat@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::repeat(x, 2)")],
    },
    SelfUpdateRow {
        function: "strings::replace@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AST,
            ATTR,
            ATTR_INIT,
            "strings::replace(x, \"a\", \"b\")",
        )],
    },
    SelfUpdateRow {
        function: "strings::upper@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::upper(x)")],
    },
    SelfUpdateRow {
        function: "strings::lower@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::lower(x)")],
    },
    SelfUpdateRow {
        function: "strings::caseFold@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::caseFold(x)")],
    },
    SelfUpdateRow {
        function: "strings::normalizeNfc@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::normalizeNfc(x)")],
    },
];
