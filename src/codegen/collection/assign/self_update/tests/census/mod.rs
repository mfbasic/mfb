//! plan-176-J: the self-update census — the arm, field-kind and site tables the
//! `self_update::tests` guards check, compiled for tests only. `SELF_UPDATE_TABLE`
//! lives in `self_update_table.rs`.

use super::*;

mod self_update_table;
pub(crate) use self_update_table::SELF_UPDATE_TABLE;

// ---------------------------------------------------------------------------
// The table. It exists for the census guards below — the dispatch reads
// `SELF_UPDATE_ARMS` only — so it is compiled for tests.
// ---------------------------------------------------------------------------

/// How a self-update-shaped function avoids copying `x`.
#[cfg(test)]
#[derive(Debug)]
pub(crate) enum SelfUpdate {
    /// In place at every site, by these arm ids (a function may need two, e.g.
    /// `append` single-element and bulk).
    Arm(&'static [ArmId]),
    /// No copy of `x` exists to avoid: the result is not built from `x`'s block
    /// (plan-142-E). `proof` cites the lowering that shows `x` is only read.
    Exempt {
        reason: &'static str,
        proof: &'static str,
    },
    /// Still copies; the named plan owns it (plan-146-A Open Decision 1: the
    /// `AttributedString` forms, `attributed-string`). plan-146-H deleted
    /// `Pending` again, as plan-142-I did: a `String` builtin with a self-update
    /// form now needs an arm or a proven exemption, never a promise.
    Deferred(&'static str),
}

/// A program fragment that performs one self-update of `x`, for the matrix test.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Probe {
    /// Packages the fragment needs besides `io`.
    pub(crate) imports: &'static [&'static str],
    /// Top-level helper `FUNC`s the call refers to (callbacks), or empty.
    pub(crate) helpers: &'static str,
    /// `x`'s declared type.
    pub(crate) ty: &'static str,
    /// `x`'s initial value.
    pub(crate) init: &'static str,
    /// The right-hand side of `x = …`.
    pub(crate) call: &'static str,
}

/// One row: a self-update-shaped function and how it is served.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct SelfUpdateRow {
    /// `pkg::name` of a registry function, or an operator spelling (`&`) for an
    /// operator self-update, which has no registry entry.
    pub(crate) function: &'static str,
    pub(crate) kind: SelfUpdate,
    /// Fragments exercising the row; together they must fire every arm it names.
    pub(crate) probes: &'static [Probe],
}

#[cfg(test)]
const C: &[&str] = &["collections"];
#[cfg(test)]
const M: &[&str] = &["math"];
#[cfg(test)]
const Z: &[&str] = &["compress", "encoding"];
#[cfg(test)]
const K: &[&str] = &["crypto", "encoding"];
#[cfg(test)]
const ST: &[&str] = &["strings"];
#[cfg(test)]
const FS: &[&str] = &["fs"];
#[cfg(test)]
const OS: &[&str] = &["os"];
#[cfg(test)]
const EN: &[&str] = &["encoding"];
#[cfg(test)]
const NET: &[&str] = &["net"];
#[cfg(test)]
const RE: &[&str] = &["regex"];
#[cfg(test)]
const AS: &[&str] = &["astrings"];
#[cfg(test)]
const AST: &[&str] = &["astrings", "strings"];

#[cfg(test)]
const fn probe(
    imports: &'static [&'static str],
    ty: &'static str,
    init: &'static str,
    call: &'static str,
) -> Probe {
    Probe {
        imports,
        helpers: "",
        ty,
        init,
        call,
    }
}

#[cfg(test)]
const fn probe_with(
    imports: &'static [&'static str],
    helpers: &'static str,
    ty: &'static str,
    init: &'static str,
    call: &'static str,
) -> Probe {
    Probe {
        imports,
        helpers,
        ty,
        init,
        call,
    }
}

/// plan-146-A: a `String` probe.
#[cfg(test)]
const fn str_probe(
    imports: &'static [&'static str],
    init: &'static str,
    call: &'static str,
) -> Probe {
    probe(imports, "String", init, call)
}

#[cfg(test)]
const STR: &str = "\"abcdef\"";
#[cfg(test)]
const PATH: &str = "\"/tmp/a/b.txt\"";
#[cfg(test)]
const ATTR: &str = "AttributedString";
#[cfg(test)]
const ATTR_INIT: &str = "astrings::fromString(\"abcdef\")";
#[cfg(test)]
const LI: &str = "List OF Integer";
#[cfg(test)]
const LF: &str = "List OF Float";
#[cfg(test)]
const LX: &str = "List OF Fixed";
#[cfg(test)]
const FIXEDS: &str = "[1.5F, 2.25F, 0.5F]";
#[cfg(test)]
const LB: &str = "List OF Byte";
#[cfg(test)]
const SI: &str = "Set OF Integer";
#[cfg(test)]
const MSI: &str = "Map OF String TO Integer";
#[cfg(test)]
const FLOATS: &str = "[0.1, 0.2, 0.3]";
#[cfg(test)]
const BYTES: &str = "encoding::utf8Encode(\"hello, hello, hello\")";
#[cfg(test)]
const IS_POSITIVE: &str = "FUNC isPositive(n AS Integer) AS Boolean\n  RETURN n > 0\nEND FUNC\n\n";
#[cfg(test)]
const NEGATED: &str = "FUNC negated(n AS Integer) AS Integer\n  RETURN 0 - n\nEND FUNC\n\n";
#[cfg(test)]
const KEYLEN: &str = "FUNC keyLen(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\n";
#[cfg(test)]
const KEYSTR: &str = "FUNC keyStr(n AS Integer) AS String\n  RETURN toString(0 - n)\nEND FUNC\n\n";
#[cfg(test)]
const PUSH: &str = "FUNC push(acc AS List OF Integer, n AS Integer) AS List OF Integer\n  RETURN collections::append(acc, n)\nEND FUNC\n\n";

#[cfg(test)]
const REDUCE_REASON: &str =
    "The result is built from `initial`, not from `x`: the fold only walks `x`.";
#[cfg(test)]
const REDUCE_PROOF: &str = "`lower_collection_reduce_impl` (`builtins/collections/gen_memory.rs`) stores `args[0]` in `reduce_collection` and only walks it (`initialize_collection_loop_slots`, `load_collection_loop_item`); the accumulator starts from `args[1]`.";
#[cfg(test)]
const COMPRESS_REASON: &str =
    "The result is a new byte stream whose length is unrelated to `x`'s; `x` is only read.";
#[cfg(test)]
const COMPRESS_PROOF: &str = "`Body::Rewrite` to an MFBASIC helper that reads its `data` parameter only through `len`, `collections::get`/`getOr` and a header-sized `mid` (`helper_deflate_core.rs`, `helper_inflate_core.rs`, `helper_gzip_frame.rs`, `helper_zlib_frame.rs`, `helper_crc32.rs`, `helper_adler32.rs`): no binding of `data`, so no copy.";
#[cfg(test)]
const ARGON_REASON: &str = "The result is a derived key; the password is only read.";
#[cfg(test)]
const ARGON_PROOF: &str = "`__crypto_argon2H0` hashes `header || password || tail` with `__crypto_blake2b3`, which compresses whole blocks of the password where they lie (`helper_blake2b.rs`); it used to concatenate the password into a buffer (fixed by plan-142-E).";
#[cfg(test)]
const SHAKE_REASON: &str = "The result is a derived digest; `data` is only read.";
#[cfg(test)]
const SHAKE_PROOF: &str = "`__crypto_keccakSponge` absorbs whole blocks straight from `data` and builds only the final padded block (`helper_keccak_sponge.rs`); it used to copy all of `data` into a padded buffer (fixed by plan-142-E).";

// plan-146-F: the `String` rows that earn `Exempt`.
#[cfg(test)]
const CODEC_REASON: &str =
    "The result is a new byte stream the codec appends to as it reads `x`; `x` is only read.";
#[cfg(test)]
const NOT_DERIVED_REASON: &str =
    "The result is not a function of `x`'s bytes: `x` only names what to read.";

#[cfg(test)]
impl ArmId {
    /// The stack-slot type names only this arm allocates (plan-141 Appendix C:
    /// each occurs exactly once in `src/`). The slot's presence in a function
    /// proves the arm fired there. `Set` has one per collection kind.
    ///
    /// plan-145-B: a field-capable arm also lists its field route's slots, one per
    /// container (`inplace_recfield_*` for a record, `inplace_state_*` for a
    /// `STATE` payload). The single and bulk `append` share their field slot, as
    /// `insert` and `prepend` share their item slot: the record and `STATE` arms
    /// they absorbed did, and the names are in the `.ncode` (byte identity).
    pub(crate) fn markers(self) -> &'static [&'static str] {
        match self {
            ArmId::Append => &[
                "inplace_append_item",
                "inplace_recfield_rhs",
                "inline_state_rhs",
            ],
            ArmId::BulkAppend => &[
                "inplace_bulk_append_rhs",
                "inplace_recfield_rhs",
                "inline_state_rhs",
            ],
            ArmId::SetAdd => &[
                "inplace_set_add_item",
                "inplace_recfield_add_item",
                "inplace_state_add_item",
            ],
            ArmId::Set => &[
                "inplace_set_index",
                "inplace_set_key",
                "inplace_recfield_set_index",
                "inplace_recfield_set_key",
                "inplace_state_set_index",
                "inplace_state_set_key",
            ],
            ArmId::RemoveKey => &[
                "inplace_remove_key",
                "inplace_recfield_remove_key",
                "inplace_state_remove_key",
            ],
            ArmId::Prepend => &[
                "inplace_prepend_item",
                "inplace_recfield_splice_item",
                "inplace_state_splice_item",
            ],
            ArmId::RemoveAt => &[
                "inplace_remove_at_index",
                "inplace_recfield_remove_at_index",
                "inplace_state_remove_at_index",
            ],
            ArmId::Insert => &[
                "inplace_insert_index",
                "inplace_recfield_splice_index",
                "inplace_state_splice_index",
            ],
            ArmId::SetRemove => &[
                "inplace_set_remove_item",
                "inplace_recfield_set_remove",
                "inplace_state_set_remove",
            ],
            ArmId::Concat => &["concat_self_right"],
            ArmId::Filter => &["inplace_filter_action"],
            ArmId::Take => &["inplace_take_count"],
            ArmId::Drop => &["inplace_drop_count"],
            ArmId::Mid => &["inplace_mid_start"],
            ArmId::Distinct => &["inplace_distinct_count"],
            ArmId::Math => &["inplace_math_result"],
            ArmId::Replace => &["inplace_replace_old"],
            ArmId::Transform => &["inplace_transform_action"],
            ArmId::Sort => &["inplace_sort_count"],
            ArmId::SortBy => &["inplace_sortby_action"],
            ArmId::Union => &["inplace_union_other"],
            ArmId::Intersection => &["inplace_intersection_other"],
            ArmId::Difference => &["inplace_difference_other"],
            ArmId::SymmetricDifference => &["inplace_symmetricDifference_other"],
            ArmId::Merge => &["inplace_merge_prefer"],
            ArmId::MapValues => &["inplace_mapvalues_action"],
            ArmId::StrIdentity => &["inplace_str_identity"],
            ArmId::StrWindow => &["inplace_str_window"],
            ArmId::StrGrow => &["inplace_str_grow"],
            ArmId::StrRewrite => &["inplace_str_rewrite"],
            ArmId::Repeat => &["inplace_repeat_item"],
        }
    }
}

/// plan-145-A: how a record field of a given type is laid out, which decides the
/// only in-place lowering a self-update of that field can have.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FieldKindClass {
    /// Stored by value in its 8-byte slot (`!record_field_is_inlined &&
    /// !record_field_is_pointer`): the scalars, an enum, `Nothing`, a resource
    /// handle.
    Scalar,
    /// The slot holds an owned pointer to its own block (`record_field_is_pointer`
    /// and not inlined): `json::Json` and the records that hold one.
    Pointer,
    /// Inlined into the owner's data region, with a size fixed at compile time:
    /// every field is `Scalar` or itself `InlinedFixed`.
    InlinedFixed,
    /// Inlined, but its size depends on its value: it holds a `String`, a
    /// collection, a data union, or an `InlinedVariable` record.
    InlinedVariable,
    /// A `List`, `Map` or `Set` — `cases.tsv`'s arms.
    Collection,
}

/// Classify `type_` as a record field (plan-145-A census). `model` must know the
/// record types `type_` reaches.
#[cfg(test)]
pub(crate) fn field_kind_class(model: &TypeModel, type_: &ParameterType) -> FieldKindClass {
    use crate::codegen::collection::layout::{record_field_is_inlined, record_field_is_pointer};
    if matches!(
        type_,
        ParameterType::ListOf(_) | ParameterType::MapOf(..) | ParameterType::SetOf(_)
    ) {
        return FieldKindClass::Collection;
    }
    if !record_field_is_inlined(model, type_) {
        return if record_field_is_pointer(model, type_) {
            FieldKindClass::Pointer
        } else {
            FieldKindClass::Scalar
        };
    }
    match model.record_fields.get(type_) {
        Some(fields)
            if fields.iter().all(|(_, field)| {
                matches!(
                    field_kind_class(model, field),
                    FieldKindClass::Scalar | FieldKindClass::InlinedFixed
                )
            }) =>
        {
            FieldKindClass::InlinedFixed
        }
        _ => FieldKindClass::InlinedVariable,
    }
}

/// plan-145-A: a field kind's in-place status (`FIELD_KIND_TABLE`).
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum FieldKindRow {
    /// Updated in place; the plan-145 letter that lands it (`C` scalar and pointer
    /// stores, `F` fixed-size overwrites, `E` the last collection arms).
    Arm(char),
    /// Rebuilt by design; the proof says why no in-place form exists.
    Rebuild {
        reason: &'static str,
        proof: &'static str,
    },
    /// Owned by another plan.
    Deferred(&'static str),
}

/// plan-145-A Open Decision 3: an inlined field whose size depends on its value.
#[cfg(test)]
const SIZE_VARIES: FieldKindRow = FieldKindRow::Rebuild {
    reason: "size-varies",
    proof: "the new value's size is known only after it is built, and an inlined field \
            without a capacity word cannot take a larger value in place; reallocating the \
            owner's tail instead copies its prefix, which costs what the rebuild does",
};

/// plan-145-A Open Decision 1: a `String` field (and an `AttributedString`, whose
/// text is one) needs a `String` arm at a field, which neither plan-145 nor
/// plan-146 lands; the follow-up plan written after both owns it (plan-146-A Open
/// Decision 1). `field_kinds.tsv` and `field_expect.tsv` spell it `deferred:string`.
#[cfg(test)]
const STRING_PLAN: FieldKindRow = FieldKindRow::Deferred("string");

/// plan-145-A: one row per field kind a record can hold (`field_kind_census`
/// enumerates them from the compiler). A new package record type fails
/// `field_kind_census_covers_every_record_field_type` until it has a row, and so
/// does a row whose kind no longer exists. The black-box twin is
/// `tests/guards/inplace_self_update_census.rs`, over `field_kinds.tsv`.
#[cfg(test)]
pub(crate) const FIELD_KIND_TABLE: &[(&str, FieldKindRow)] = &[
    ("Integer", FieldKindRow::Arm('C')),
    ("Float", FieldKindRow::Arm('C')),
    ("Fixed", FieldKindRow::Arm('C')),
    ("Money", FieldKindRow::Arm('C')),
    ("Boolean", FieldKindRow::Arm('C')),
    ("Byte", FieldKindRow::Arm('C')),
    ("String", STRING_PLAN),
    ("AttributedString", STRING_PLAN),
    ("json.Json", FieldKindRow::Arm('C')),
    ("List OF Integer", FieldKindRow::Arm('E')),
    ("Map OF String TO Integer", FieldKindRow::Arm('E')),
    ("Set OF Integer", FieldKindRow::Arm('E')),
    ("astrings.AttrFlag", FieldKindRow::Arm('F')),
    ("astrings.AttrText", SIZE_VARIES),
    ("astrings.AttrNumber", FieldKindRow::Arm('F')),
    ("audio.AudioDevice", SIZE_VARIES),
    ("audio.AudioEnvelope", FieldKindRow::Arm('F')),
    ("audio.AudioNote", FieldKindRow::Arm('F')),
    ("big.Int", SIZE_VARIES),
    ("big.DivResult", SIZE_VARIES),
    ("canvas.MouseEvent", SIZE_VARIES),
    ("canvas.Size", FieldKindRow::Arm('F')),
    ("canvas.TextMetrics", FieldKindRow::Arm('F')),
    // plan-162-D/E: two scalar fields each, the TextMetrics shape.
    ("canvas.CaretStop", FieldKindRow::Arm('F')),
    ("strings.LineBreak", FieldKindRow::Arm('F')),
    ("canvas.FrameStats", FieldKindRow::Arm('F')),
    ("canvas.Transform", FieldKindRow::Arm('F')),
    ("canvas.GradientStop", FieldKindRow::Arm('F')),
    ("canvas.Gradient", SIZE_VARIES),
    ("canvas.Paint", SIZE_VARIES),
    ("canvas.Rectangle", SIZE_VARIES),
    ("canvas.Line", SIZE_VARIES),
    ("canvas.Polygon", SIZE_VARIES),
    ("canvas.Circle", SIZE_VARIES),
    ("canvas.Arc", SIZE_VARIES),
    ("canvas.Text", SIZE_VARIES),
    ("canvas.Picture", SIZE_VARIES),
    // plan-173: an image handle and four `vector::Float2`s, no `id` -- fixed size.
    ("canvas.Tile", FieldKindRow::Arm('F')),
    ("canvas.AutoPaint", SIZE_VARIES),
    ("canvas.Group", SIZE_VARIES),
    ("canvas.Ellipse", SIZE_VARIES),
    ("canvas.PolyLine", SIZE_VARIES),
    ("canvas.BezierCurve", SIZE_VARIES),
    ("canvas.PolyCurve", SIZE_VARIES),
    ("canvas.CurvePolygon", SIZE_VARIES),
    ("canvas.DrawLayer", SIZE_VARIES),
    ("canvas.ParticleSystem", SIZE_VARIES),
    ("canvas.Emission", FieldKindRow::Arm('F')),
    ("canvas.Motion", FieldKindRow::Arm('F')),
    ("canvas.Appearance", FieldKindRow::Arm('F')),
    ("json.JsonNull", FieldKindRow::Arm('F')),
    ("json.JsonBool", FieldKindRow::Arm('F')),
    ("json.JsonNum", FieldKindRow::Arm('F')),
    ("json.JsonStr", SIZE_VARIES),
    ("json.JsonArr", FieldKindRow::Arm('C')),
    ("json.JsonObj", FieldKindRow::Arm('C')),
    ("regex.Group", SIZE_VARIES),
    ("regex.MatchInfo", SIZE_VARIES),
    ("io.Key", SIZE_VARIES),
    ("term.TermSize", FieldKindRow::Arm('F')),
    ("term.MouseEvent", FieldKindRow::Arm('F')),
    ("datetime.Instant", FieldKindRow::Arm('F')),
    ("datetime.Duration", FieldKindRow::Arm('F')),
    ("datetime.Date", FieldKindRow::Arm('F')),
    ("datetime.Time", FieldKindRow::Arm('F')),
    ("datetime.Zone", SIZE_VARIES),
    ("datetime.DateTime", SIZE_VARIES),
    ("crypto.Sealed", SIZE_VARIES),
    ("crypto.KeyPair", SIZE_VARIES),
    ("udp.Datagram", SIZE_VARIES),
    ("vector.Float2", FieldKindRow::Arm('F')),
    ("vector.Float3", FieldKindRow::Arm('F')),
    ("vector.Float4", FieldKindRow::Arm('F')),
    ("vector.Fixed2", FieldKindRow::Arm('F')),
    ("vector.Fixed3", FieldKindRow::Arm('F')),
    ("vector.Fixed4", FieldKindRow::Arm('F')),
    ("vector.Integer2", FieldKindRow::Arm('F')),
    ("vector.Integer3", FieldKindRow::Arm('F')),
    ("vector.Integer4", FieldKindRow::Arm('F')),
    ("http.Response", SIZE_VARIES),
    ("http.PendingState", SIZE_VARIES),
    ("http.Request", SIZE_VARIES),
    ("http.RequestPart", SIZE_VARIES),
    // bug-781: its `handler` is a function value the record owns, so the record is a
    // pointer field (not inlined) and is replaced by arm C's pointer store.
    ("http.Route", FieldKindRow::Arm('C')),
    ("net.Url", SIZE_VARIES),
    ("net.Address", SIZE_VARIES),
    ("net.PingResult", SIZE_VARIES),
    ("color.Color", FieldKindRow::Arm('F')),
    ("color.Hsl", FieldKindRow::Arm('F')),
];

/// plan-147-B: `(arm, reason)` — the arms that never fire at `Site::Return` (S11),
/// nor at `Site::OwnedParam` (S12, S11 inside an owned variant), nor at `Site::S11F`
/// (S11's field form), by design. The matrix asserts they do NOT fire there, and does not require them
/// to; an arm that starts firing fails the matrix, so the exclusion cannot rot.
///
/// Every entry is a `String` arm, and they all share one cause. A `String` block
/// that leaves its frame must be **tight**: `arena_free(ptr, size)` is caller-sized
/// and bins by size class (`arena.rs:lower_arena_free`), and a caller frees a
/// returned `String` by its `byteLength` alone — so a block still carrying capacity
/// spare cannot be moved out. That is bug-560, which `plan_returned_move` encodes as
/// its `string_capacity_slots` decline. Every in-place `String` update leaves spare
/// by construction (a window shrinks `byteLength` inside the old allocation; a grow
/// takes geometric headroom), so at a `RETURN` the block has to be copied tight —
/// and that copy costs exactly the one allocation the copying builtin would have
/// made anyway.
///
/// **Measured** (plan-147-B Phase 2, 2026-09-22), the `chain` shape over
/// `strings::left(x, 150)` under `mfb build --debug`, summed
/// `arena.<k>.alloc_calls`:
///
/// | | N = 600 | 2N = 1200 |
/// |---|---|---|
/// | arm fires (a shadow pre-allocated at the `RETURN`) | 603 | 1203 |
/// | arm declines (HEAD) | 603 | 1203 |
///
/// Identical. S11 is a measured no-op for `String`, so the arms are excluded rather
/// than given a shadow that would buy a stack slot and nothing else.
#[cfg(test)]
pub(crate) const RETURN_NEVER: &[(ArmId, &str)] = &[
    (
        ArmId::StrWindow,
        "a window shrinks byteLength inside the old allocation, leaving spare; a \
         String block must be tight to move out of its frame (bug-560)",
    ),
    (
        ArmId::StrGrow,
        "a grow takes geometric headroom, leaving spare; a String block must be \
         tight to move out of its frame (bug-560)",
    ),
    (
        ArmId::StrRewrite,
        "a rewrite keeps the old allocation, which is only tight when the rewrite \
         is length-preserving; a String block must be tight to move out (bug-560)",
    ),
    (
        ArmId::Concat,
        "`RETURN s & t` is a `Binary`, not a call, so it is not a `RETURN OP(x, …)` \
         shape at all — and the grown block could not move out either (bug-560)",
    ),
];

/// plan-147-F: `(arm, reason)` — the arms that never fire at `Site::OwnedParam` (S12)
/// on top of [`RETURN_NEVER`]'s.
///
/// S12 needs an owned-parameter VARIANT to lower in, and a bare `String` parameter is
/// never handed over, so no `String` arm has a variant to fire in — `handOver$own1`
/// is not emitted for a `String` at all. `RETURN_NEVER` already covers every other
/// `String` arm for the bug-560 tightness reason; `StrIdentity` (`x = toString(x)`,
/// which emits nothing) is the one that still fires at `Site::Return`, where the
/// owner is a LOCAL, and cannot at S12.
///
/// The refusal is in `handover_type`: a `String` binding may hold a pointer to a
/// static literal rather than an arena block, and the variant's owned-parameter free
/// would be a bus error (measured as exit 138; plan-147-F Corrections). A `String`
/// FIELD of a record is unaffected, which is why `S11F` is not listed here.
#[cfg(test)]
pub(crate) const OWNED_PARAM_NEVER: &[(ArmId, &str)] = &[(
    ArmId::StrIdentity,
    "a bare `String` parameter is never handed over (its block may be a static \
     literal), so there is no owned variant for S12 to lower the arm in",
)];

/// The binding sites the matrix test compiles every arm probe at: plan-142's four
/// plain sites, and plan-145's fifteen field sites (plan-144's audit legend).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    /// S1 — a `MUT` local in a function body.
    Local,
    /// S7 — a `MUT` local inside a `FOR EACH` over itself (plan-142-F). A `String`
    /// is not a collection, so no `FOR EACH` walks one: the `&` row has no S7.
    ForEach,
    /// S9 — a `MUT` captured by reference in a `collections::forEach` lambda
    /// (plan-142-G). The self-update lowers in the lifted lambda. plan-146-G gave a
    /// by-ref `String` the owner's capacity shadow, shared through the closure
    /// environment, so every `String` arm and `&` fire here too.
    Lambda,
    /// S2 — a module-level `MUT` global (plan-142-H), self-updated in a `FUNC … AS Nothing`.
    Global,
    /// S11 — `RETURN OP(x, …)` on an owned local at its last use (plan-147-B). The
    /// probe is a recursive `chain`, one level per call, so the block threads
    /// through every level instead of being rebuilt: a copying `RETURN` allocates
    /// once per level.
    Return,
    /// S11F — the FIELD form of S11 (plan-147-F): `RETURN WITH r { b := OP(r.b, …) }`
    /// on an owned record. The probe threads the record through a helper, exactly as
    /// the census's `state = walkSchema(…, state, …)` does, so the self-update lowers
    /// inside the helper's owned variant against the record's own block.
    S11F,
    /// S12 — `RETURN OP(x, …)` on an owned PARAMETER, inside the owned variant an
    /// approved caller calls (plan-147-E). The probe is `x = handOver(x, …)` in a
    /// loop: the caller hands `x` over, so inside `handOver$own1` the parameter is an
    /// owned local and S11 applies to it. The self-update lowers in the VARIANT, not
    /// in `handOver` itself, which is what `lowers_in` pins.
    ///
    /// The helper is NOT called `step`, as plan-147-E §3 wrote it: `STEP` is a
    /// keyword (`FOR i = 1 TO 10 STEP 2`) and keywords are case-insensitive, so
    /// `FUNC step(…)` is `MFB_PARSE_INVALID_IDENTIFIER`.
    OwnedParam,
    /// A local record's not-last field: `r = WITH r { a := f(r.a, …) }`.
    S3,
    /// The same record's last field `b`.
    S4,
    /// The last field of a module-level record, updated in a `FUNC … AS Nothing`.
    S5,
    /// `b` of `inner AS Rec` in `Out { n, inner }`, through a nested `WITH`.
    S6,
    /// S4 inside a `FOR EACH` over the field.
    S7,
    /// S4 in a `collections::forEach` lambda capturing the record.
    S9,
    /// S4 plus a scalar update in the same `WITH` (`RecN { a, b, n }`).
    S10,
    /// Field `a` of an owner handle's `STATE` payload `P { a, b, n }`.
    T1,
    /// Field `b` of the same payload.
    T2,
    /// T1 through a resource parameter (`FUNC run1(RES h …) AS Nothing`).
    T3,
    /// T2 through a resource parameter.
    T4,
    /// Two updates over the whole payload (`h.state = WITH h.state { b := …, n := … }`).
    T5,
    /// `b` of `inner AS PIn` in `Q { inner, n }`.
    T6,
    /// T2 inside a `FOR EACH` over the field.
    T7,
    /// T2 on a resource-union handle `RES h AS Stream STATE P`.
    T8,
}

#[cfg(test)]
pub(crate) const ENABLED_SITES: &[Site] = &[
    Site::Local,
    Site::ForEach,
    Site::Lambda,
    Site::Global,
    Site::Return,
    Site::OwnedParam,
];

/// plan-145-A: the field sites the matrix compiles every arm probe at.
#[cfg(test)]
pub(crate) const FIELD_SITES: &[Site] = &[
    Site::S3,
    Site::S4,
    Site::S5,
    Site::S6,
    Site::S7,
    Site::S9,
    Site::S10,
    Site::T1,
    Site::T2,
    Site::T3,
    Site::T4,
    Site::T5,
    Site::T6,
    Site::T7,
    Site::T8,
    // plan-147-F: the field form of S11.
    Site::S11F,
];

/// plan-145-A: `(arm, probe type or "" for every probe, field sites, reason)` — the
/// probes an arm never fires for at those sites, by design. The matrix asserts
/// they do not fire, and does not require them to.
#[cfg(test)]
pub(crate) const FIELD_NEVER: &[(ArmId, &str, &[&str], &str)] = {
    const NOT_LAST: &[&str] = &["S3", "T1", "T3"];
    /// plan-146-B: every field site — a `String` arm serves none.
    const ALL_FIELD_SITES: &[&str] = &[
        "S3", "S4", "S5", "S6", "S7", "S9", "S10", "T1", "T2", "T3", "T4", "T5", "T6", "T7", "T8",
        // plan-147-F: S11's field form is a field site too, so plan-145's own field
        // exclusions (a `String` field is `deferred:string`, a grow at a not-last
        // field would shift its sibling) apply there unchanged.
        "S11F",
    ];
    const NOT_LAST_GROW: &str = "a grow at a not-last field would shift the next sibling \
                                 (plan-145-E non-goal)";
    &[
        (ArmId::Append, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::BulkAppend, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::SetAdd, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::Insert, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::Prepend, "", NOT_LAST, NOT_LAST_GROW),
        (
            ArmId::Set,
            "Map OF String TO Integer",
            NOT_LAST,
            NOT_LAST_GROW,
        ),
        (ArmId::Union, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::SymmetricDifference, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::Merge, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::Repeat, "", NOT_LAST, NOT_LAST_GROW),
        (
            ArmId::Concat,
            "",
            ALL_FIELD_SITES,
            "a `String` field is deferred:string (plan-145-A Open Decision 1)",
        ),
        (
            ArmId::StrIdentity,
            "",
            ALL_FIELD_SITES,
            "a `String` field is deferred:string (plan-145-A Open Decision 1)",
        ),
        (
            ArmId::StrWindow,
            "",
            ALL_FIELD_SITES,
            "a `String` field is deferred:string (plan-145-A Open Decision 1)",
        ),
        (
            ArmId::StrGrow,
            "",
            ALL_FIELD_SITES,
            "a `String` field is deferred:string (plan-145-A Open Decision 1)",
        ),
        (
            ArmId::StrRewrite,
            "",
            ALL_FIELD_SITES,
            "a `String` field is deferred:string (plan-145-A Open Decision 1)",
        ),
    ]
};

#[cfg(test)]
impl Site {
    /// Whether the self-update at this site lowers in the function `name`.
    pub(crate) fn lowers_in(self, name: &str) -> bool {
        match self {
            Site::Local | Site::ForEach => name == "main",
            Site::Lambda | Site::S9 => name.starts_with("$lambda"),
            Site::Global | Site::S5 | Site::T3 | Site::T4 => name == "run1",
            // plan-147-B: the self-update is the `chain` function's own `RETURN`.
            Site::Return => name == "chain",
            // plan-147-E: it lowers in the owned VARIANT of `step`, never in `step`
            // itself — the base lowering still lends its parameter.
            Site::OwnedParam => name.starts_with("handOver$own"),
            // plan-147-F: likewise, in the record helper's owned variant.
            Site::S11F => name.starts_with("handOverRec$own"),
            _ => name == "main",
        }
    }

    /// The site's code in `FIELD_NEVER` and the harness tables (`S4`, `T2`, …).
    pub(crate) fn code(self) -> String {
        format!("{self:?}")
    }

    /// The field a field site updates.
    fn field(self) -> &'static str {
        match self {
            Site::S3 => "r.a",
            Site::S5 => "gR.b",
            Site::S6 => "o.inner.b",
            Site::T1 | Site::T3 => "h.state.a",
            Site::T2 | Site::T4 | Site::T5 | Site::T7 | Site::T8 => "h.state.b",
            Site::T6 => "h.state.inner.b",
            _ => "r.b",
        }
    }
}

/// `text` with every whole-word `from` replaced by `to` (outside string literals).
#[cfg(test)]
fn replace_word(text: &str, from: &str, to: &str) -> String {
    let bytes = text.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut i = 0;
    while i < text.len() {
        if bytes[i] == b'"' {
            in_string = !in_string;
        }
        if !in_string
            && text[i..].starts_with(from)
            && (i == 0 || !word(bytes[i - 1]))
            && bytes.get(i + from.len()).is_none_or(|b| !word(*b))
        {
            out.push_str(to);
            i += from.len();
            continue;
        }
        let c = text[i..].chars().next().expect("in bounds");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[cfg(test)]
impl Probe {
    /// The whole program performing this probe's self-update at `site`, in `main`;
    /// `None` when `site` has no form for the probe's type.
    pub(crate) fn source(&self, site: Site) -> Option<String> {
        if FIELD_SITES.contains(&site) {
            return self.field_source(site);
        }
        let mut src = String::from("IMPORT io\n");
        for import in self.imports {
            src.push_str(&format!("IMPORT {import}\n"));
        }
        // S9 calls `collections::forEach`.
        if matches!(site, Site::Lambda) && !self.imports.contains(&"collections") {
            src.push_str("IMPORT collections\n");
        }
        src.push('\n');
        src.push_str(self.helpers);
        match site {
            Site::Local => src.push_str(&format!(
                "FUNC main() AS Integer\n  MUT x AS {ty} = {init}\n  FOR i = 1 TO 3\n    \
                 x = {call}\n  NEXT\n  io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            // plan-146-G: `Site::Lambda` (S9) is a `String` site now — the lambda
            // shares the owner's capacity shadow. `FOR EACH` still cannot walk a
            // `String` (`TYPE_FOR_EACH_REQUIRES_COLLECTION`).
            Site::ForEach if self.ty == "String" => return None,
            Site::Lambda if self.ty == "AttributedString" => return None,
            Site::ForEach if self.ty == "AttributedString" => return None,
            Site::ForEach => src.push_str(&format!(
                "FUNC main() AS Integer\n  MUT x AS {ty} = {init}\n  FOR EACH each1 IN x\n    \
                 x = {call}\n  NEXT\n  io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            Site::Global => src.push_str(&format!(
                "MUT x AS {ty} = {init}\n\nFUNC run1() AS Nothing\n  FOR i = 1 TO 3\n    x = {call}\n  \
                 NEXT\nEND FUNC\n\nFUNC main() AS Integer\n  run1()\n  \
                 io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            Site::Lambda => src.push_str(&format!(
                "FUNC main() AS Integer\n  MUT x AS {ty} = {init}\n  \
                 LET one AS List OF Integer = [0]\n  FOR i = 1 TO 3\n    \
                 collections::forEach(one, LAMBDA(each1 AS Integer) -> x = {call})\n  \
                 NEXT\n  io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            // plan-147-E (S12): `x = handOver(x, …)` in a loop. The caller hands `x`
            // over, so inside `handOver`'s owned variant the parameter is an owned
            // local and S11 applies to it. Everything the statement needs is a
            // literal, so the helper takes only `x`.
            Site::OwnedParam => src.push_str(&format!(
                "FUNC handOver(x AS {ty}) AS {ty}\n  RETURN {call}\nEND FUNC\n\n\
                 FUNC main() AS Integer\n  MUT x AS {ty} = {init}\n  FOR i = 1 TO 3\n    \
                 x = handOver(x)\n  NEXT\n  io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            // plan-147-B (S11): the `chain` shape of plan-147-B §3 step 4. One
            // level per call, so the copying lowering allocates once per level and
            // the in-place one only the arm's geometric growth.
            Site::Return => src.push_str(&format!(
                "FUNC chain(k AS Integer) AS {ty}\n  IF k = 0 THEN RETURN {init}\n  \
                 MUT x AS {ty} = chain(k - 1)\n  RETURN {call}\nEND FUNC\n\n\
                 FUNC main() AS Integer\n  LET r AS {ty} = chain(3)\n  \
                 io::print(toString(len(r)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            _ => unreachable!("field sites return above"),
        }
        Some(src)
    }

    /// plan-145-A: the probe's self-update applied to a field at `site`: the same
    /// templates as the runtime harness (`tests/runtime/rt_inplace_self_update.rs`).
    fn field_source(&self, site: Site) -> Option<String> {
        let ty = self.ty;
        if ty == "String" && matches!(site, Site::S7 | Site::T7) {
            return None;
        }
        // plan-147-F: S11F is not one of plan-145's owner shapes — it threads the
        // record through a helper — so it writes its own program rather than going
        // through the owner/looped assembly below.
        if site == Site::S11F {
            let v = replace_word(self.call, "x", "r.b");
            return Some(self.field_program(
                site,
                &format!(
                    "FUNC handOverRec(r AS Rec) AS Rec\n  RETURN WITH r {{ b := {v} }}\nEND FUNC\n\n\
                     FUNC main() AS Integer\n  MUT x AS {ty} = {init}\n  \
                     MUT r AS Rec = Rec[a := x, b := x]\n  FOR i = 1 TO 3\n    \
                     r = handOverRec(r)\n  NEXT\n  io::print(toString(len(r.b)))\n  \
                     RETURN 0\nEND FUNC\n",
                    init = self.init
                ),
            ));
        }
        let field = site.field();
        let v = replace_word(self.call, "x", field);
        let statement = match site {
            Site::S3 => format!("r = WITH r {{ a := {v} }}"),
            Site::S5 => format!("gR = WITH gR {{ b := {v} }}"),
            Site::S6 => format!("o = WITH o {{ inner := WITH o.inner {{ b := {v} }} }}"),
            Site::S10 => format!("r = WITH r {{ b := {v}, n := k }}"),
            Site::T5 => format!("h.state = WITH h.state {{ b := {v}, n := k }}"),
            Site::T6 => format!("h.state.inner = WITH h.state.inner {{ b := {v} }}"),
            Site::T1 | Site::T2 | Site::T3 | Site::T4 | Site::T7 | Site::T8 => {
                format!("{field} = {v}")
            }
            _ => format!("r = WITH r {{ b := {v} }}"),
        };
        let owner = match site {
            Site::S10 => "  MUT r AS RecN = RecN[a := x, b := x, n := 0]\n",
            Site::S6 => "  MUT o AS Out = Out[n := 1, inner := Rec[a := x, b := x]]\n",
            Site::S5 => "  gR = Rec[a := x, b := x]\n",
            Site::T6 => "  h.state = Q[inner := PIn[a := x, b := x], n := 0]\n",
            Site::T1 | Site::T2 | Site::T3 | Site::T4 | Site::T5 | Site::T7 | Site::T8 => {
                "  h.state = P[a := x, b := x, n := 0]\n"
            }
            _ => "  MUT r AS Rec = Rec[a := x, b := x]\n",
        };
        let looped = match site {
            Site::S7 | Site::T7 => {
                format!("  FOR EACH each1 IN {field}\n    {statement}\n  NEXT\n")
            }
            Site::S9 => format!(
                "  LET one AS List OF Integer = [0]\n  FOR i = 1 TO 3\n    \
                 collections::forEach(one, LAMBDA(each1 AS Integer) -> {statement})\n  NEXT\n"
            ),
            _ => format!("  FOR i = 1 TO 3\n    {statement}\n  NEXT\n"),
        };
        // S10/T5's second update is `n := k`, a local read (plan-144's site legend).
        let k = if matches!(site, Site::S10 | Site::T5) {
            "  LET k AS Integer = 7\n"
        } else {
            ""
        };
        let body = format!(
            "  MUT x AS {ty} = {init}\n{owner}{k}{looped}  io::print(toString(len({field})))\n",
            init = self.init
        );
        let open = "fs::openFile(\"/dev/null\")";
        let program = match site {
            Site::S5 => format!(
                "MUT gR AS Rec\n\nFUNC run1() AS Nothing\n{body}END FUNC\n\n\
                 FUNC main() AS Integer\n  run1()\n  RETURN 0\nEND FUNC\n"
            ),
            Site::T3 | Site::T4 => format!(
                "FUNC run1(h AS fs::File STATE P) AS Nothing\n{body}END FUNC\n\n\
                 FUNC main() AS Integer\n  RES h AS fs::File STATE P = {open}\n  run1(h)\n  \
                 RETURN 0\nEND FUNC\n"
            ),
            Site::T6 => format!(
                "FUNC main() AS Integer\n  RES h AS fs::File STATE Q = {open}\n{body}  \
                 RETURN 0\nEND FUNC\n"
            ),
            Site::T8 => format!(
                "FUNC main() AS Integer\n  RES h AS Stream STATE P = {open}\n{body}  \
                 RETURN 0\nEND FUNC\n"
            ),
            Site::T1 | Site::T2 | Site::T5 | Site::T7 => format!(
                "FUNC main() AS Integer\n  RES h AS fs::File STATE P = {open}\n{body}  \
                 RETURN 0\nEND FUNC\n"
            ),
            _ => format!("FUNC main() AS Integer\n{body}  RETURN 0\nEND FUNC\n"),
        };
        Some(self.field_program(site, &program))
    }

    /// The shared prelude of a field probe — imports, the line's helpers, and
    /// plan-145's record types — with `program` appended.
    fn field_program(&self, site: Site, program: &str) -> String {
        let ty = self.ty;
        let mut src = String::from("IMPORT io\n");
        let mut imports: Vec<&str> = self.imports.to_vec();
        imports.push("fs");
        imports.push("collections");
        if site == Site::T8 {
            imports.push("tcp");
        }
        let mut seen = Vec::new();
        for import in imports {
            if !seen.contains(&import) {
                seen.push(import);
                src.push_str(&format!("IMPORT {import}\n"));
            }
        }
        src.push('\n');
        src.push_str(self.helpers);
        src.push_str(&format!(
            "TYPE Rec\n  a AS {ty}\n  b AS {ty}\nEND TYPE\n\n\
             TYPE RecN\n  a AS {ty}\n  b AS {ty}\n  n AS Integer\nEND TYPE\n\n\
             TYPE Out\n  n AS Integer\n  inner AS Rec\nEND TYPE\n\n\
             TYPE P\n  a AS {ty}\n  b AS {ty}\n  n AS Integer\nEND TYPE\n\n\
             TYPE PIn\n  a AS {ty}\n  b AS {ty}\nEND TYPE\n\n\
             TYPE Q\n  inner AS PIn\n  n AS Integer\nEND TYPE\n\n"
        ));
        if site == Site::T8 {
            src.push_str("UNION Stream\n  fs::File\n  tcp::Socket\nEND UNION\n\n");
        }
        src.push_str(program);
        src
    }
}
