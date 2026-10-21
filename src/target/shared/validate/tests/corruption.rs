//! `validate_nir` refuses a module whose declarations are malformed, with a message
//! naming the declaration.
//!
//! The front end cannot produce any of these -- a function with no name, an
//! import of kind `proc`, a union declared twice -- so each is a corrupted
//! module, the way a bad package merge or a lowering bug would hand one over. The
//! message is the contract: it is what `-nir` validation prints for the build
//! failure, and it must name what is wrong rather than say "invalid".

use super::*;
use crate::target::shared::nir::{NirImport, NirImportParam};

fn refusal(module: &NirModule) -> String {
    validate_nir(module).expect_err("the corrupted module is refused")
}

fn import(name: &str) -> NirImport {
    NirImport {
        package: "pkg".to_string(),
        name: name.to_string(),
        symbol: format!("_mfb_pkg_{name}"),
        kind: "func".to_string(),
        isolated: false,
        params: vec![NirImportParam {
            type_: ParameterType::Integer,
            has_default: false,
        }],
        returns: ParameterType::Integer,
    }
}

#[test]
fn an_empty_target_os_or_architecture_is_refused() {
    for (os, arch) in [("", "x86_64"), ("linux", "")] {
        let target = crate::target::BuildTarget {
            os: os.to_string(),
            arch: arch.to_string(),
        };
        assert_eq!(
            validate_target(&target).err().as_deref(),
            Some("native target must include an OS and architecture")
        );
    }
}

#[test]
fn a_runtime_helper_declared_twice_is_refused() {
    let module = module(vec![RuntimeHelper::Io, RuntimeHelper::Io]);
    assert_eq!(
        refusal(&module),
        format!(
            "NIR runtime helper '{}' is declared more than once",
            RuntimeHelper::Io.name()
        )
    );
}

#[test]
fn a_type_of_an_unknown_kind_is_refused_by_name() {
    let mut module = module(vec![RuntimeHelper::Io]);
    let mut odd = record_type("Odd", &[]);
    odd.kind = "bogus".to_string();
    module.types.push(odd);
    assert_eq!(refusal(&module), "NIR type 'Odd' has invalid kind 'bogus'");
}

#[test]
fn a_function_with_a_bad_kind_visibility_or_duplicate_name_is_refused() {
    let mut module = module(vec![RuntimeHelper::Io]);
    module.functions[0].kind = "proc".to_string();
    assert_eq!(
        refusal(&module),
        "NIR function 'main' has invalid kind 'proc'"
    );
    module.functions[0].kind = "func".to_string();
    module.functions[0].visibility = "secret".to_string();
    assert_eq!(
        refusal(&module),
        "NIR function 'main' has invalid visibility 'secret'"
    );
    module.functions[0].visibility = "private".to_string();
    let twin = super::module(Vec::new()).functions.remove(0);
    module.functions.push(twin);
    assert_eq!(
        refusal(&module),
        "NIR function 'main' is declared more than once"
    );
}

#[test]
fn an_import_with_an_empty_name_a_bad_kind_or_a_twin_is_refused() {
    let mut module = module(vec![RuntimeHelper::Io]);
    let mut blank = import("scale");
    blank.symbol = String::new();
    module.imports = vec![blank];
    assert_eq!(
        refusal(&module),
        "NIR import package, name, and symbol must not be empty"
    );
    let mut proc_ = import("scale");
    proc_.kind = "proc".to_string();
    module.imports = vec![proc_];
    assert_eq!(
        refusal(&module),
        "NIR import 'scale' has invalid kind 'proc'"
    );
    module.imports = vec![import("scale"), import("scale")];
    assert_eq!(
        refusal(&module),
        "NIR import 'scale' is declared more than once"
    );
}
