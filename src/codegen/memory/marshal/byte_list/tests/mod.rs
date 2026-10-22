//! byte_list.rs's tests (moved out of the file, Corrections C15 of `planning/tests.md`:
//! a suite inside the file is measured as part of it).

// bug-564: a marshal helper is spliced INTO its caller's body, and that body
// numbers its own vregs from `%v0` with a `Vregs`. A helper that names a fixed
// `%vN` therefore writes whichever caller vreg happens to carry that number.
// `emit_free_buffer_guarded`'s `%v9` was Schannel `tls::connect`'s live
// host-string pointer, and the second connect in a Windows process crashed.
//
// Two pins. The scan below keeps the whole module free of literal vreg names
// (a new helper cannot reintroduce the class by copy-paste). The behavioural
// test drives every helper with a caller that already holds 32 vregs and
// asserts no emitted operand is one of them.
use super::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::util::Vregs;

/// Every `"%v<digits>"` string literal in `source` outside its test module.
fn literal_vreg_names(source: &str) -> Vec<String> {
    let code = source.split("#[cfg(test)]").next().unwrap_or(source);
    let mut found = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("\"%v") {
        let tail = &rest[at + 3..];
        let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() && tail[digits.len()..].starts_with('"') {
            found.push(format!("%v{digits}"));
        }
        rest = &rest[at + 3..];
    }
    found
}

#[test]
fn no_marshal_helper_names_a_fixed_vreg() {
    for (file, source) in [
        ("byte_list.rs", include_str!("../../byte_list.rs")),
        ("record.rs", include_str!("../../record.rs")),
        ("record_list.rs", include_str!("../../record_list.rs")),
    ] {
        let names = literal_vreg_names(source);
        assert!(
            names.is_empty(),
            "{file} names {} fixed vreg(s) outside its tests ({:?}…); a helper spliced \
             into a caller's body must mint its vregs from the caller's `Vregs`",
            names.len(),
            &names[..names.len().min(6)]
        );
    }
}

/// Every rendered operand of every instruction.
fn operands(instructions: &[CodeInstruction]) -> Vec<String> {
    instructions
        .iter()
        .flat_map(|i| {
            i.fields
                .iter()
                .map(|(key, _)| i.get(key).unwrap_or_default())
        })
        .collect()
}

#[test]
fn every_helper_mints_its_vregs_from_the_caller() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let mut vregs = Vregs::new();
    let held: Vec<String> = (0..32).map(|_| vregs.next().to_string()).collect();
    let mut ins = Vec::new();
    let mut rel = Vec::new();
    emit_read_byte_list("s", "r", 8, 16, 24, "f", &mut vregs, &mut ins, &mut rel);
    emit_build_byte_list(
        "s",
        "bl",
        "bd",
        16,
        24,
        Some(32),
        abi::mfb_return(1),
        "f",
        &mut vregs,
        &mut ins,
        &mut rel,
    );
    emit_zero_guarded("s", 16, Some(24), 0, "z", &mut vregs, &mut ins);
    emit_zero_guarded("s", 16, None, 64, "zc", &mut vregs, &mut ins);
    emit_free_byte_list_guarded("s", "w", 32, 24, true, &mut vregs, &mut ins, &mut rel);
    emit_free_byte_list_guarded("s", "n", 32, 24, false, &mut vregs, &mut ins, &mut rel);
    emit_free_buffer_guarded("s", "b", 16, ScratchSize::Slot(24), &mut ins, &mut rel);
    let used = operands(&ins);
    assert!(
        used.iter().any(|o| o.starts_with("%v")),
        "the helpers must emit vregs at all, or this test cannot fail"
    );
    for operand in used {
        assert!(
            !held.contains(&operand),
            "a marshal helper wrote `{operand}`, a vreg its caller already held"
        );
    }
}
