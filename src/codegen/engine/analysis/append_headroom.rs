//! bug-770 C: which `MUT x = collections::get/getOr(…)` bindings may receive
//! their owned copy WITH growth headroom instead of shrink-to-fit.
//!
//! A `get`/`getOr` result is an owned copy (`mfb spec language memory-semantics`
//! §14.6), and a copy is tight (`copy_collection_tight`: `capacity == count`).
//! When the very next statement is the in-place self-update
//! `x = collections::append(x, item)`, that tight copy is always full, so the
//! append reallocates at once — a second allocation and a second copy of the
//! whole list, per bucket update in the `listagg` shape
//! (`bucket = getOr(m, k, empty)`, `append`, `set`).
//!
//! Headroom is a property of a mutable WORKING BUFFER, never of a value (`mfb spec
//! memory collections`, *Capacity Headroom and Growth*). A binding whose next
//! statement appends to it in place IS such a buffer: that append would give it
//! exactly this headroom (`capacity = step(count)`), so copying with it up front
//! makes the append's grow and the copy one event. After the append the block has
//! the shape the tight copy + grow gave it, so nothing downstream of the pair
//! changes: every store of the binding (`set` into a container, return, field,
//! global, thread transfer) still sees a buffer the in-place append produced.
//!
//! The recognizer is deliberately narrow and fails closed: the binding must be a
//! `MUT` of a `List` type initialised by a plain `get`/`getOr` call, and the op
//! IMMEDIATELY after it in the same block must be `x = append(x, item)` with `x`
//! as the first argument. Anything else — an intervening statement, the append
//! under an `IF`, a different builtin — keeps the tight copy. The builder adds the
//! type gates it alone can answer (`CodeBuilder::append_headroom_bind`): a
//! single-element item, a flat (non-graph) element, a local that is not
//! address-taken. Keys are `op_key`s of the `Bind` ops, so a lowering that clones
//! a body simply finds no match and copies tight.

use crate::codegen::engine::analysis::last_use::op_key;
use crate::target::shared::nir::visit::{walk_ops, NirVisitor};
use crate::target::shared::nir::{NirOp, NirValue};
use crate::types::ParameterType;
use std::collections::HashMap;

/// The approved binds, keyed by the `Bind` op's `op_key`, each with the append's
/// item operand (the builder checks it is a single element, not a bulk list).
pub(crate) type AppendHeadroomBinds = HashMap<usize, NirValue>;

fn builtin(target: &str) -> Option<&'static str> {
    crate::codegen::builtins::native_builtin_target(target)
}

/// `x = append(x, item)` → `Some(item)`.
fn self_append_item<'a>(op: &'a NirOp, name: &str) -> Option<&'a NirValue> {
    let NirOp::Assign {
        name: target,
        value:
            NirValue::Call {
                target: callee,
                args,
                ..
            },
    } = op
    else {
        return None;
    };
    if target != name || builtin(callee) != Some("append") || args.len() != 2 {
        return None;
    }
    match &args[0] {
        NirValue::Local(first) if first == name => Some(&args[1]),
        _ => None,
    }
}

/// `MUT x AS List OF … = get/getOr(…)` → `Some(x)`.
fn headroom_candidate(op: &NirOp) -> Option<&str> {
    let NirOp::Bind {
        mutable: true,
        name,
        type_: ParameterType::ListOf(_),
        value: Some(NirValue::Call { target, .. }),
    } = op
    else {
        return None;
    };
    matches!(builtin(target), Some("get" | "getOr")).then_some(name.as_str())
}

/// Every `get`/`getOr` → `MUT` bind in `ops` (at any depth) whose next statement
/// is an `append` self-update of the same local.
pub(crate) fn collect_append_headroom_binds(ops: &[NirOp]) -> AppendHeadroomBinds {
    struct Collector {
        out: AppendHeadroomBinds,
    }
    impl NirVisitor for Collector {
        fn visit_ops(&mut self, ops: &[NirOp]) {
            for pair in ops.windows(2) {
                if let Some(name) = headroom_candidate(&pair[0]) {
                    if let Some(item) = self_append_item(&pair[1], name) {
                        self.out.insert(op_key(&pair[0]), item.clone());
                    }
                }
            }
            walk_ops(self, ops);
        }
    }
    let mut collector = Collector {
        out: AppendHeadroomBinds::new(),
    };
    collector.visit_ops(ops);
    collector.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{nir_for_src, CodeTarget};

    /// The names of `main`'s approved binds, sorted.
    fn approved(source: &str) -> Vec<String> {
        let module = nir_for_src(
            source,
            CodeTarget::MacosAarch64,
            crate::target::NativeBuildMode::Console,
        )
        .unwrap_or_else(|error| panic!("the probe lowers to NIR: {error}"));
        let main = module
            .functions
            .iter()
            .find(|f| f.name == "main" || f.name.rsplit(['.', ':']).next() == Some("main"))
            .expect("main");
        let keys = collect_append_headroom_binds(&main.body);
        struct Names<'k> {
            keys: &'k AppendHeadroomBinds,
            out: Vec<String>,
        }
        impl NirVisitor for Names<'_> {
            fn visit_op(&mut self, op: &NirOp) {
                if let NirOp::Bind { name, .. } = op {
                    if self.keys.contains_key(&op_key(op)) {
                        self.out.push(name.clone());
                    }
                }
                crate::target::shared::nir::visit::walk_op(self, op);
            }
        }
        let mut names = Names {
            keys: &keys,
            out: Vec::new(),
        };
        names.visit_ops(&main.body);
        names.out.sort();
        names.out
    }

    fn program(body: &str) -> String {
        format!(
            "IMPORT io
IMPORT collections

FUNC main() AS Integer
  MUT m AS Map OF Integer TO List OF Integer = Map OF Integer TO List OF Integer {{}}
  MUT rs AS List OF List OF Integer = [[1], [2]]
  LET empty AS List OF Integer = []
{body}  RETURN 0
END FUNC
"
        )
    }

    #[test]
    fn a_get_or_get_bind_appended_by_the_next_statement_is_approved() {
        let got = approved(&program(
            "  FOR i = 0 TO 9
    MUT b AS List OF Integer = collections::getOr(m, i MOD 3, empty)
    b = collections::append(b, i)
    m = collections::set(m, i MOD 3, b)
    MUT r AS List OF Integer = collections::get(rs, 1)
    r = collections::append(r, i)
    rs = collections::set(rs, 1, r)
  NEXT
",
        ));
        assert_eq!(got, ["b", "r"]);
    }

    #[test]
    fn every_other_shape_keeps_the_tight_copy() {
        let got = approved(&program(
            "  LET l AS List OF Integer = collections::getOr(m, 1, empty)
  MUT gap AS List OF Integer = collections::getOr(m, 1, empty)
  io::print(toString(len(gap)))
  gap = collections::append(gap, 1)
  MUT branch AS List OF Integer = collections::getOr(m, 1, empty)
  IF len(branch) = 0 THEN
    branch = collections::append(branch, 1)
  END IF
  MUT other AS List OF Integer = collections::getOr(m, 1, empty)
  other = collections::prepend(other, 1)
  MUT plain AS List OF Integer = empty
  plain = collections::append(plain, 1)
  MUT cross AS List OF Integer = collections::getOr(m, 1, empty)
  gap = collections::append(cross, 1)
  io::print(toString(len(l) + len(gap) + len(branch) + len(other) + len(plain) + len(cross)))
",
        ));
        assert!(got.is_empty(), "approved {got:?}");
    }
}
