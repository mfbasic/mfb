//! `strings.displayWidth` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

const INTRO: &str = r#"Measure the terminal column width of a string."#;

const DESC: &str = r#"`strings::displayWidth` returns the number of terminal columns `value` occupies
when printed to a fixed-width (monospace) terminal. It is the sum, over the
string's extended grapheme clusters, of each cluster's display width.

Each cluster contributes `0`, `1`, or `2` columns. A cluster's width is the width
of its first non-zero-width scalar: `0` for a cluster made only of zero-width
scalars (a lone combining mark, a zero-width space, or a zero-width joiner), `2`
for a cluster led by an East Asian Wide or Fullwidth character or an
emoji-presentation symbol, and `1` for everything else. The per-scalar width
comes from the vendored utf8proc `charwidth` table.

The string is first segmented into extended grapheme clusters using the same
UAX #29 boundary rules as `strings::graphemes`, so a base letter with combining
marks, a regional-indicator flag, or a zero-width-joiner emoji family each counts
as one cluster laid out in its lead scalar's width.

Display width is therefore a fourth measure, distinct from `len(value)` (Unicode
scalar values), `strings::byteLen(value)` (UTF-8 bytes), and
`strings::graphemesCount(value)` (grapheme clusters). For CJK text, emoji, or
combining sequences all four can differ: `"日本語"` is three clusters and three
scalars but six display columns, while `"café"` written with a combining accent
(`"cafe"` plus `U+0301`) is four clusters and four display columns but five
scalars.

East Asian **Ambiguous**-width characters are treated as width `1` (narrow), the
modern terminal default. The empty string yields `0`. `value` is not mutated and
the call never fails.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Wide CJK ideographs occupy two columns each:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::displayWidth("日本語")))
  io::print(toString(strings::displayWidth("abc")))
  RETURN 0
END FUNC
```

Zero-width and combining scalars do not add columns:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET nfd AS String = "cafe" & "́"
  io::print(toString(strings::displayWidth(nfd)))
  io::print(toString(len(nfd)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() == 1 {
        if let Some(value) = builder.static_string_value_vr(&args[0]) {
            let width: i64 = crate::unicode::backend::graphemes(&value)
                .iter()
                .map(|cluster| {
                    cluster
                        .chars()
                        .map(|c| {
                            crate::unicode::runtime_tables::property_for_codepoint(c as u32)
                                .charwidth() as i64
                        })
                        .find(|w| *w != 0)
                        .unwrap_or(0)
                })
                .sum();
            return builder.lower_value(&NirValue::Const {
                type_: ParameterType::Integer,
                value: width.to_string(),
            });
        }
    }
    if args.len() != 1 {
        return Err("strings.displayWidth: no native lowering for these arguments".to_string());
    }
    let value = &args[0];

    let ptr = builder.temporary_vreg();
    let len = builder.temporary_vreg();
    let data = builder.temporary_vreg();
    let pos = builder.temporary_vreg();
    let total = builder.temporary_vreg();
    let cluster_w = builder.temporary_vreg();
    let cp = builder.temporary_vreg();
    let adv = builder.temporary_vreg();
    let prop = builder.temporary_vreg();
    let bc_prev = builder.temporary_vreg();
    let icb_prev = builder.temporary_vreg();
    let bc_cur = builder.temporary_vreg();
    let icb_cur = builder.temporary_vreg();
    let cw = builder.temporary_vreg();
    let addr = builder.temporary_vreg();
    let (ptr, len, data, pos, total, cluster_w) = (&ptr, &len, &data, &pos, &total, &cluster_w);
    let (cp, adv, prop, bc_prev, icb_prev, bc_cur, icb_cur, cw, addr) = (
        &cp, &adv, &prop, &bc_prev, &icb_prev, &bc_cur, &icb_cur, &cw, &addr,
    );
    let value = value.clone();
    builder.require_string("strings.displayWidth value", &value)?;
    let value_slot = builder.spill_to_slot("strings_display_width_value", &value.location);

    let empty = builder.label("strings_display_width_empty");
    let walk = builder.label("strings_display_width_loop");
    let is_break = builder.label("strings_display_width_break");
    let no_break = builder.label("strings_display_width_no_break");
    let after = builder.label("strings_display_width_after");
    let skip_set = builder.label("strings_display_width_skip_set");
    let loop_done = builder.label("strings_display_width_loop_done");
    let done = builder.label("strings_display_width_done");

    builder.emit(abi::load_u64(ptr, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(len, ptr, 0));
    builder.emit(abi::compare_immediate(len, "0"));
    builder.emit(abi::branch_eq(&empty));
    builder.emit(abi::add_immediate(data, ptr, 8));
    builder.emit(abi::move_immediate(total, "Integer", "0"));
    // Seed: decode the first scalar, prime the grapheme state, and set the
    // first cluster's width from it (cluster_w starts 0, so this is the
    // first-non-zero-width rule for scalar 0).
    builder.emit_utf8_decode_next(data, cp, adv);
    builder.emit_unicode_property_lookup(cp, prop);
    builder.emit_unicode_property_boundclass(prop, bc_prev);
    builder.emit_unicode_property_indic_conjunct_break(prop, icb_prev);
    builder.emit_unicode_property_charwidth(prop, cw);
    builder.emit(abi::move_register(cluster_w, cw));
    builder.emit(abi::move_register(pos, adv));
    builder.emit(abi::label(&walk));
    builder.emit(abi::compare_registers(pos, len));
    builder.emit(abi::branch_ge(&loop_done));
    builder.emit(abi::add_registers(addr, data, pos));
    builder.emit_utf8_decode_next(addr, cp, adv);
    builder.emit_unicode_property_lookup(cp, prop);
    builder.emit_unicode_property_boundclass(prop, bc_cur);
    builder.emit_unicode_property_indic_conjunct_break(prop, icb_cur);
    builder.emit_unicode_property_charwidth(prop, cw);
    builder.emit_grapheme_break_branch(bc_prev, icb_prev, bc_cur, icb_cur, &is_break, &no_break);
    // A boundary ends the current cluster: flush its width and start fresh so
    // the current scalar seeds the new cluster below.
    builder.emit(abi::label(&is_break));
    builder.emit(abi::add_registers(total, total, cluster_w));
    builder.emit(abi::move_immediate(cluster_w, "Integer", "0"));
    builder.emit(abi::branch(&after));
    builder.emit(abi::label(&no_break));
    builder.emit(abi::label(&after));
    // First-non-zero-width rule: if this cluster has no width yet, take this
    // scalar's. cw==0 for a combining mark, so a zero-width scalar never
    // overrides a base already seen.
    builder.emit(abi::compare_immediate(cluster_w, "0"));
    builder.emit(abi::branch_ne(&skip_set));
    builder.emit(abi::move_register(cluster_w, cw));
    builder.emit(abi::label(&skip_set));
    builder.emit_grapheme_state_update(bc_prev, icb_prev, bc_cur, icb_cur);
    builder.emit(abi::add_registers(pos, pos, adv));
    builder.emit(abi::branch(&walk));
    builder.emit(abi::label(&loop_done));
    builder.emit(abi::add_registers(total, total, cluster_w));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(total, "Integer", "0"));
    builder.emit(abi::label(&done));

    let result = builder.allocate_register();
    builder.emit(abi::move_register(&result, total));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(result.render()),
        text: "strings.displayWidth".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "displayWidth",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string whose display width is measured. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
