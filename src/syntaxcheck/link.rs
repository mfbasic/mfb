use super::*;

impl<'a> SyntaxChecker<'a> {
    /// Register native `LINK` resources declared in this package into the
    /// resource registry as `kind = native` (plan-link-update.md §9). The close
    /// op is the dotted `alias.func`; `close_may_fail` is derived from whether the
    /// close wrapper has a `SUCCESS_ON` gate; sendability comes from the
    /// declaration's `THREAD_SENDABLE` opt-in (plan-link-update.md §8).
    pub(super) fn collect_native_resources(&mut self) {
        // Map every LINK function `alias.func` to whether it can fail (has a
        // SUCCESS_ON / ERROR_ON gate).
        let mut close_may_fail: HashMap<String, bool> = HashMap::new();
        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Link(link) = item {
                    for function in &link.functions {
                        close_may_fail.insert(
                            format!("{}.{}", link.alias, function.name),
                            function.success_on.is_some(),
                        );
                    }
                }
            }
        }

        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Resource(resource) = item {
                    let close_function = resource.close_fn.clone();
                    let may_fail = close_may_fail
                        .get(&close_function)
                        .copied()
                        .unwrap_or(false);
                    self.resource_registry.register(
                        resource.name.clone(),
                        crate::codegen::resource::ResourceInfo {
                            close_function,
                            sendable: resource.thread_sendable,
                            close_may_fail: may_fail,
                            kind: crate::codegen::resource::ResourceKind::Native,
                        },
                    );
                }
            }
        }
    }

    /// Register native `LINK` function signatures (keyed `alias.func`) and any
    /// `FUNC alias AS alias::func` re-exports, so wrapper code that calls
    /// `sqliteLink::open(...)` or importers that call `sqlite::close(...)` get a
    /// type (plan-link-update.md §5a/§5b).
    pub(super) fn collect_native_functions(&mut self) {
        // First gather every LINK function's signature so aliases can adopt them.
        let mut link_sigs: HashMap<String, (FunctionSig, String)> = HashMap::new();
        for file in &self.hir.files {
            for item in &file.items {
                let HirItem::Link(link) = item else {
                    continue;
                };
                for function in &link.functions {
                    let sig = self.native_function_sig(function, &file.path);
                    let key = format!("{}.{}", link.alias, function.name);
                    self.functions
                        .entry(key.clone())
                        .or_default()
                        .push(sig.clone());
                    link_sigs.insert(key, (sig, file.path.clone()));
                }
            }
        }

        // Then register re-export aliases, adopting the target's signature with
        // the alias's declared visibility (plan-link-update.md §5a).
        for file in &self.hir.files {
            for item in &file.items {
                let HirItem::FuncAlias(alias) = item else {
                    continue;
                };
                if let Some((sig, _)) = link_sigs.get(&alias.target) {
                    let mut adopted = sig.clone();
                    adopted.visibility = alias.visibility;
                    adopted.owner_file_path = file.path.clone();
                    self.functions
                        .entry(alias.name.clone())
                        .or_default()
                        .push(adopted);
                }
            }
        }
    }

    pub(super) fn native_function_sig(
        &self,
        function: &crate::ast::LinkFunction,
        owner_file_path: &str,
    ) -> FunctionSig {
        let return_type = function
            .return_type
            .as_deref()
            .map(|name| self.parse_type(name))
            .unwrap_or(Type::Nothing);
        let params = function
            .params
            .iter()
            .map(|param| ParamSig {
                name: param.name.clone(),
                type_: param
                    .type_name
                    .as_deref()
                    .map(|name| self.parse_type(name))
                    .unwrap_or(Type::Unknown),
                has_default: param.default.is_some(),
            })
            .collect();
        FunctionSig {
            kind: FunctionKind::Func,
            params,
            return_type,
            isolated: false,
            imported_package_export: false,
            // A LINK block is package-local; its functions are reachable from any
            // file of the declaring package via the alias namespace.
            visibility: Visibility::Public,
            owner_file_path: owner_file_path.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::*;

    // Every fixture is a whole program ending in a valid `FUNC main`. The
    // LINK/CSTRUCT/ABI shapes mirror `tests/rt-behavior/native/*/src/main.mfb`
    // and the `demoLink` pattern in `helpers.rs`. Each `#[test]` drives one
    // `self.report(...)` diagnostic in this file (or an `accepts` path that
    // exercises the branch's success arms).

    // ----- RESOURCE built-in shadow ------------------------------------------

    #[test]
    fn user_resource_named_like_a_builtin_is_accepted() {
        // plan-97 / bug-441: builtin resources are package-qualified (`fs::File`), so a
        // user `RESOURCE File` (bare) names a distinct user resource and is accepted.
        // The RESOURCE_SHADOWS_BUILTIN rule that once drove this test could not fire
        // after that change and was retired (plan-107-B).
        let src = "\
RESOURCE File CLOSE BY demoLink::close

LINK \"demo\" AS demoLink
  FUNC close(RES f AS File) AS Nothing
    SYMBOL \"demo_close\"
    ABI (f CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- collect_native_resources register path (clean, accepts) ----------

    #[test]
    fn clean_native_resource_and_link_accepts() {
        // A user RESOURCE closed by a LINK function with SUCCESS_ON, plus a
        // producer, registers cleanly (close_may_fail = true from success_on).
        let src = "\
RESOURCE Db CLOSE BY demoLink::close

LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- CSTRUCT declarations / escape ------------------------------------

    #[test]
    fn duplicate_cstruct_name_is_rejected() {
        // link.rs:406 — a LINK block declaring the same CSTRUCT name twice.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn cstruct_named_in_wrapper_param_is_rejected() {
        // link.rs:370 — a wrapper parameter typed as a declared CSTRUCT.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f(x AS Foo) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn cstruct_named_in_wrapper_return_is_rejected() {
        // link.rs:384 — a wrapper returning a declared CSTRUCT.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Foo
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn cstruct_bad_field_ctype_forwards_check_cstruct_fault() {
        // link.rs:421-430 — a CSTRUCT field using an unknown ctype forwards a
        // crate::ir::check_cstruct fault, pointed at the field line.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CBogus
  END CSTRUCT
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    // ----- struct slots / BIND IN -------------------------------------------

    #[test]
    fn inout_non_struct_slot_is_rejected() {
        // link.rs:196 — a scalar slot marked INOUT whose ctype is not a CSTRUCT.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a INOUT CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn cstruct_mapping_to_non_record_is_rejected() {
        // link.rs:210 — a CSTRUCT whose MAPS target is an ENUM, not a record,
        // drives record_fields_of -> None.
        let src = "\
ENUM Color
  Red
  Green
END ENUM

LINK \"c\" AS libc
  CSTRUCT Foo AS Color
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s OUT Foo) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn returning_an_in_struct_slot_is_rejected() {
        // link.rs:240 — a wrapper that RETURNs a struct slot declared IN.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Rec
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN s
      a = 0
    END BIND
    RETURN s
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn returning_struct_slot_with_wrong_return_type_is_rejected() {
        // link.rs:251 — returns a struct slot but the wrapper return type is not
        // the CSTRUCT's mapped record.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

TYPE Other
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Other
    SYMBOL \"f\"
    ABI (s OUT Foo) AS status CInt32
    RETURN s
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn cstruct_record_field_type_disagreement_forwards_fault() {
        // link.rs:232 — check_struct_slot fault: CStruct field maps to Integer
        // but the record declares it String.
        let src = "\
TYPE Rec
  a AS String
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Rec
    SYMBOL \"f\"
    ABI (s OUT Foo) AS status CInt32
    RETURN s
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn bind_in_unknown_slot_is_rejected() {
        // link.rs:268 — BIND IN names an ABI slot that does not exist.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN nonesuch
      a = 0
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn bind_in_non_struct_slot_is_rejected() {
        // link.rs:280 — BIND IN names a slot whose ctype is not a CSTRUCT.
        let src = "\
LINK \"c\" AS libc
  FUNC f(n AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (n CInt64) AS status CInt32
    BIND IN n
      a = 0
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn bind_in_out_slot_is_rejected() {
        // link.rs:292 — BIND IN writes an OUT slot.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Rec
    SYMBOL \"f\"
    ABI (s OUT Foo) AS status CInt32
    BIND IN s
      a = 0
    END BIND
    RETURN s
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn bind_in_unknown_field_is_rejected() {
        // link.rs:305 — BIND IN sets a field the CSTRUCT does not declare.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN s
      nosuchfield = 0
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn bind_in_duplicate_field_is_rejected() {
        // link.rs:316 — BIND IN sets the same field twice.
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN s
      a = 0
      a = 1
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn bind_in_bad_value_is_rejected() {
        // link.rs:342 — BIND IN sets a field from a string literal (neither a
        // wrapper param nor an int/bool/-int literal).
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo) AS status CInt32
    BIND IN s
      a = \"hello\"
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn clean_bind_in_value_shapes_accept() {
        // link.rs:327-339 — a clean BIND IN whose values exercise the Identifier
        // (param), Number, Unary "-", and Boolean ok=true arms.
        let src = "\
TYPE Rec
  a AS Integer
  b AS Integer
  c AS Integer
  d AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
    b CInt64
    c CInt64
    d CInt64
  END CSTRUCT
  FUNC f(p AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (s IN Foo, rem CPtr) AS status CInt32
    CONST rem = NOTHING
    BIND IN s
      a = p
      b = 5
      c = -3
      d = TRUE
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- C ABI escape / ctype validity ------------------------------------

    #[test]
    fn wrapper_cptr_param_and_return_escape() {
        // link.rs:450/463 — raw C ABI types in the wrapper's MFBASIC-facing
        // signature (param and return arms of is_c_abi_type).
        let src = "\
LINK \"demo\" AS demoLink
  FUNC leak(handle AS CPtr) AS Nothing
    SYMBOL \"demo_leak\"
    ABI (handle CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
  FUNC produce() AS CPtr
    SYMBOL \"demo_produce\"
    ABI (out OUT CPtr) AS status CInt32
    RETURN out
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn bad_return_ctype_is_rejected() {
        // link.rs:494 — an ABI return ctype not in the closed table.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CBogus
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn bad_slot_ctype_both_directions_are_rejected() {
        // link.rs:511-525 — an unknown ctype in argument position (valid_as_argument)
        // and on an OUT slot (valid_as_return arm).
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CBogus, b OUT CBogus) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    // ----- CONST pins -------------------------------------------------------

    #[test]
    fn const_pin_on_out_slot_is_rejected() {
        // link.rs:537 — CONST pinning a slot that is also OUT.
        let src = "\
LINK \"c\" AS libc
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (s OUT CInt64) AS status CInt32
    CONST s = 5
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn non_foldable_const_value_is_rejected() {
        // link.rs:690 — a CONST whose value is an arbitrary identifier.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64, flag CInt64) AS status CInt32
    CONST flag = bogusIdent
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CONST_UNKNOWN_SLOT"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn clean_const_foldable_shapes_accept() {
        // link.rs:672-687 — SIZEOF <CStruct>, boolean literal, NOTHING, and a
        // negated integer all fold (the foldable=true arms).
        let src = "\
TYPE Rec
  a AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT Foo AS Rec
    a CInt64
  END CSTRUCT
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (sz CInt64, b CInt64, n CPtr, neg CInt64, s IN Foo) AS status CInt32
    CONST sz = SIZEOF Foo
    CONST b = TRUE
    CONST n = NOTHING
    CONST neg = -1
    BIND IN s
      a = 0
    END BIND
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn const_pin_on_unknown_slot_is_rejected() {
        // link.rs:711 — a CONST pinning a slot name not in the ABI.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    CONST nosuch = 5
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_CONST_UNKNOWN_SLOT"),
            "{:?}",
            check_src(src)
        );
    }

    // ----- unbound slots / params, result markers ---------------------------

    #[test]
    fn unbound_input_slot_is_rejected() {
        // link.rs:561 — an input ABI slot with no matching parameter/CONST/OUT/BIND.
        let src = "\
LINK \"c\" AS libc
  FUNC f() AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn success_on_reading_unknown_name_is_rejected() {
        // link.rs:594 — a SUCCESS_ON expression reading an identifier that names
        // no ABI slot and is not the ABI return.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON typo = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn nothing_wrapper_with_return_is_rejected() {
        // link.rs:626 — a Nothing wrapper that declares a RETURN.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    RETURN status
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn unbound_wrapper_parameter_is_rejected() {
        // link.rs:655 — a wrapper parameter with no matching ABI slot, BIND IN
        // field, or BUFFER SIZE use.
        let src = "\
LINK \"c\" AS libc
  FUNC f(a AS Integer, orphan AS Integer) AS Nothing
    SYMBOL \"f\"
    ABI (a CInt64) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        // The rejection is `ir::verify`'s (plan-107-C); this keeps the walk.
        let _ = check_src(src);
    }

    #[test]
    fn clean_cbuffer_wrappers_accept() {
        // link.rs:99-158 + 648-652 — a clean CBuffer program: check_buffer_slots
        // runs with zero faults, and `pairs` (SIZE-only) drives by_buffer_size.
        let src = "\
LINK \"c\" AS libc
  FUNC preadBytes(fd AS Integer, nbyte AS Integer, offset AS Integer) AS List OF Byte
    SYMBOL \"pread\"
    ABI (fd CInt32, buf OUT CBuffer, nbyte CInt64, offset CInt64) AS got CInt64
    BUFFER buf SIZE nbyte
    RETURN buf LENGTH got
  END FUNC
  FUNC preadPairs(fd AS Integer, nbyte AS Integer, pairs AS Integer, offset AS Integer) AS List OF Byte
    SYMBOL \"pread\"
    ABI (fd CInt32, buf OUT CBuffer, nbyte CInt64, offset CInt64) AS got CInt64
    BUFFER buf SIZE pairs * 2
    RETURN buf LENGTH got
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn cbuffer_without_return_hits_result_slot_none_arm() {
        // A buffer function with no RETURN drives the `_ => None` result_slot arm
        // of the buffer-rule view (the missing-result rejection itself is
        // `ir::verify`'s since plan-107-C, so only the walk is asserted here).
        let src = "\
LINK \"c\" AS libc
  FUNC noReturn(n AS Integer) AS List OF Byte
    SYMBOL \"f\"
    ABI (buf OUT CBuffer) AS got CInt64
    BUFFER buf SIZE n
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        let _ = check_src(src);
    }

    // ----- FREE blocks ------------------------------------------------------

    #[test]
    fn free_on_resource_producer_is_rejected() {
        // link.rs:735 (+ early return :743) — a FREE block on an `AS RES` producer.
        let src = "\
RESOURCE Db CLOSE BY libc::close

LINK \"sqlite3\" AS libc
  FUNC open(path AS String) AS RES Db
    SYMBOL \"sqlite3_open\"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    FREE db
      SYMBOL \"sqlite3_free\"
      ABI (ptr CPtr) AS CVoid
    END FREE
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"sqlite3_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_FREE_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn malformed_free_block_is_rejected() {
        // link.rs:770 — a FREE whose deallocator does not return CVoid.
        let src = "\
LINK \"sqlite3\" AS sql
  FUNC expandedSql(stmt AS Integer) AS String
    SYMBOL \"sqlite3_expanded_sql\"
    ABI (stmt CInt64) AS text CPtr
    RETURN text
    FREE text
      SYMBOL \"sqlite3_free\"
      ABI (ptr CPtr) AS CInt32
    END FREE
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "NATIVE_FREE_INVALID"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn well_formed_free_block_accepts() {
        // link.rs:745-767 — a well-formed FREE (CPtr return surfaced by RETURN,
        // deallocator taking one CPtr and returning CVoid) drives the ok=true path.
        let src = "\
LINK \"sqlite3\" AS sql
  FUNC expandedSql(stmt AS Integer) AS String
    SYMBOL \"sqlite3_expanded_sql\"
    ABI (stmt CInt64) AS text CPtr
    RETURN text
    FREE text
      SYMBOL \"sqlite3_free\"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    // ----- re-exports / signatures ------------------------------------------

    #[test]
    fn func_alias_reexport_adopts_link_signature() {
        // link.rs:808-823 + 826-860 — a `FUNC open AS demoLink::open` alias adopts
        // the LINK signature; a bare `demoLink::open(...)` call drives
        // native_function_sig's param/return parse_type mapping.
        let src = "\
RESOURCE Db CLOSE BY demoLink::close

LINK \"demo\" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL \"demo_open\"
    ABI (path CString, db OUT CPtr) AS status CInt32
    RETURN db
    SUCCESS_ON status = 0
  END FUNC
  FUNC close(RES db AS Db) AS Nothing
    SYMBOL \"demo_close\"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

FUNC open AS demoLink::open

FUNC main AS Integer
  RES db AS Db = open(\":memory:\")
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }
}
