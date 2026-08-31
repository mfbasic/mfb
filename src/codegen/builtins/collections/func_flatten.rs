//! `collections::flatten` — descriptor entry + `.mfb` body + native fast path.
//!
//! Owns everything for `flatten`: docs, the `Implementation::Mfb` fallback body
//! (BODY, byte-significant 2-space indent — do not reformat), and the native
//! accelerator ([`CodeBuilder::flatten_fast_path`]) wired in via
//! `mfb_with_fast_path`. The fast path self-gates on the `#collections_flatten$T`
//! monomorph target and either lowers natively or declines (`Ok(None)`), in which
//! case the codegen seam monomorphizes BODY instead.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::typed_list_element_type;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
/// plan-86 A3: native `collections::flatten` (`#collections_flatten$T`, 1 arg)
/// for a simple result element T (String or fixed-width) — the inner lists are
/// inline self-contained blocks, bulk-appended into the result with no per-inner
/// copy. A nested `List OF List OF List ...` (T itself a list) or any other shape
/// declines (`Ok(None)`), falling through to the `.mfb` body.
///
/// A free function, not a method: an `impl` method does not coerce to the
/// higher-ranked `MfbFastPath` fn-pointer type (E0308), the same reason the
/// `Native` lowerings are free functions.
pub(crate) fn flatten_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(t) = target.strip_prefix("#collections_flatten$") else {
        return Ok(None);
    };
    if !(matches!(t, "String" | "Integer" | "Float" | "Fixed" | "Money") && args.len() == 1) {
        return Ok(None);
    }
    builder.lower_flatten_native(args).map(Some)
}

impl CodeBuilder<'_> {
    fn lower_flatten_native(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let source = self.lower_value(&args[0])?;
        let outer_type = source.type_.clone();
        let inner_type = typed_list_element_type(&outer_type)
            .cloned()
            .ok_or_else(|| format!("native flatten does not accept {outer_type}"))?;
        let elem = typed_list_element_type(&inner_type)
            .cloned()
            .ok_or_else(|| format!("native flatten inner type {inner_type} is not a list"))?;
        let source_slot = self.allocate_stack_object("flatten_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));
        // outerCount = count(source)
        let oc_slot = self.allocate_stack_object("flatten_outer_count", 8);
        let r0 = self.temporary_vreg();
        let r1 = self.temporary_vreg();
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&r1, &r0, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r1, abi::stack_pointer(), oc_slot));
        // result = empty, growable List OF <elem>
        let result = self.lower_empty_collection(&inner_type)?;
        let result_slot = self.allocate_stack_object("flatten_result", 8);
        self.emit(abi::store_u64(
            &result.location,
            abi::stack_pointer(),
            result_slot,
        ));
        let inner_slot = self.allocate_stack_object("flatten_inner_ptr", 8);
        let i_slot = self.allocate_stack_object("flatten_i", 8);
        let loop_l = self.label("flatten_loop");
        let done_l = self.label("flatten_done");
        self.emit(abi::move_immediate(&r0, "Integer", "0"));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), i_slot));
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&r1, abi::stack_pointer(), oc_slot));
        self.emit(abi::compare_registers(&r0, &r1));
        self.emit(abi::branch_ge(&done_l));
        // (voff, vlen) = outer entry i; innerPtr = outerDataBase + voff.
        let voff = self.temporary_vreg();
        let vlen = self.temporary_vreg();
        let sc1 = self.temporary_vreg();
        let sc2 = self.temporary_vreg();
        let ob = self.temporary_vreg();
        let db = self.temporary_vreg();
        self.emit(abi::load_u64(&ob, abi::stack_pointer(), source_slot));
        self.emit_element_value_offset(&voff, &vlen, &ob, &r0, &sc1, &sc2, &inner_type);
        self.emit(abi::load_u64(&ob, abi::stack_pointer(), source_slot));
        self.emit_collection_data_pointer_for(&db, &ob, &inner_type);
        self.emit(abi::add_registers(&db, &db, &voff));
        self.emit(abi::store_u64(&db, abi::stack_pointer(), inner_slot));
        // result = bulk-append(result, inner) — concatenates the inner's elements.
        self.lower_list_bulk_append_in_place(result_slot, inner_slot, &inner_type, &elem)?;
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), i_slot));
        self.emit(abi::add_immediate(&r0, &r0, 1));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result_reg = self.allocate_register();
        self.emit(abi::load_u64(
            &result_reg,
            abi::stack_pointer(),
            result_slot,
        ));
        Ok(ValueResult {
            origin: None,
            type_: inner_type.clone(),
            location: Operand::from(result_reg.render()),
            text: format!("flatten({outer_type})"),
        })
    }
}

// --- source-generic descriptor + body ---

const INTRO: &str = r#"Concatenate a list of lists into a single list, one level deep"#;

const DESC: &str = r#"`collections::flatten` walks `value` from index 0 upward and concatenates each
inner list onto an accumulating result. It does this by calling
`collections::append(result, inner)` where `inner` is itself a `List OF T` — that
is the list-concatenation overload of `append`, which accepts a second argument
that is either the element type or the same list type as the first argument.
Each inner list is therefore spliced in whole rather than nested as a single
element.

`flatten` removes exactly **one** level of nesting. Its parameter type is
`List OF List OF T`, so applying it to a `List OF List OF List OF Integer`
produces a `List OF List OF Integer` — the innermost lists survive as elements.
Flattening further requires calling `flatten` again on the result. It is not
recursive and there is no depth parameter.

Order is fully preserved: the inner lists are read in their own order, and
the items within each inner list keep their relative order, so the result reads
as the inner lists laid end to end. Empty inner lists contribute nothing and are
simply skipped over; they do not produce a placeholder element. When `value`
itself is empty, the result is an empty list.

`value` is not modified, and neither are the inner lists it holds; the result is
a newly built list. `flatten` invokes no user callback and raises no error.

Note that the template argument `T` is inferred from the argument, so a bare
untyped `[]` literal cannot be passed directly — bind it to a
`List OF List OF T` first, or pass an expression whose type is known.

`flatten` is generic over a single template parameter `T`, the element type of
the inner lists. It is inferred from the argument, which must be a
`List OF List OF T`; a plain `List OF T` does not match, and a doubly nested
`List OF List OF List OF T` binds `T` to `List OF ...` and so flattens only its
outermost level."#;

const EX: &str = r#"Concatenate three inner lists:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET nested AS List OF List OF Integer = [[1, 2], [3], [4, 5]]
  LET flat AS List OF Integer = collections::flatten(nested)
  io::print(toString(len(flat)))
  RETURN 0
END FUNC
```

Empty inner lists contribute nothing:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET nested AS List OF List OF String = [["a"], [], ["b", "c"]]
  LET flat AS List OF String = collections::flatten(value := nested)
  io::print(collections::get(flat, 1))
  RETURN 0
END FUNC
```

Only one level is removed, so flattening twice is two calls:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET deep AS List OF List OF List OF Integer = [[[1, 2], [3]],]
  LET once AS List OF List OF Integer = collections::flatten(deep)
  LET twice AS List OF Integer = collections::flatten(once)
  io::print(toString(len(once)) & " " & toString(len(twice)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __collections_flatten OF T(value AS List OF List OF T) AS List OF T
  MUT result AS List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    LET inner AS List OF T = collections::get(value, i)
    result = collections::append(result, inner)
    i = i + 1
  END WHILE
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "flatten",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF List OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list of inner lists to concatenate. May be empty, and any inner list may be empty. Not modified.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::list_of(ParameterType::var("T"))),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::mfb_with_fast_path(BODY, "__collections_flatten", flatten_fast_path),
        }],
    });
}
