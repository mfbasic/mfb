//! Working a location out against a thread: memory that reads short, an
//! offset past the address space, a vector register, and the `lt` and `in`
//! tests of a condition on a value that can be read.

use std::ffi::OsString;

use super::*;
use crate::debugger::{
    Ending, FunctionRange, FunctionSymbol, Register, RegisterGroup, RunStop, Step, ThreadFacts,
};

/// One thread with `x0` = -2 and a vector `v0`; memory at 0x20 holds 7,
/// and a read at 0x10 comes back short.
struct Thread;

impl Inferior for Thread {
    fn pid(&self) -> u32 {
        1
    }
    fn arch(&self) -> &'static crate::debugger::arch::Arch {
        &crate::debugger::arm64::AARCH64
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(Ending::Killed)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        Ok(Vec::new())
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        match address {
            0x10 => Ok(vec![0; 4]),
            0x20 => Ok(7u64.to_le_bytes()[..length].to_vec()),
            _ => Err(format!("unmapped {address:#x}")),
        }
    }
    fn read_code(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn function_at(&mut self, _: u64) -> Result<Option<FunctionRange>, String> {
        Ok(None)
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        Ok(Vec::new())
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        Ok(vec![
            Register {
                name: "x0".to_string(),
                group: RegisterGroup::General,
                value: RegisterValue::Bits64((-2_i64) as u64),
                role: None,
            },
            Register {
                name: "v0".to_string(),
                group: RegisterGroup::Simd,
                value: RegisterValue::Bits128(1),
                role: None,
            },
        ])
    }
    fn set_registers(&mut self, _: usize, _: &[(String, u64)]) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
        Err("read-only".to_string())
    }
    fn restart(&mut self, _: &[(OsString, OsString)]) -> Result<u32, String> {
        Err("read-only".to_string())
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn insert_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn remove_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn insert_watchpoint(&mut self, _: u64, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn remove_watchpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        Err("read-only".to_string())
    }
}

fn value(word: &str) -> Result<u64, Fault> {
    Location::parse(word)
        .unwrap()
        .value(&mut Thread, 0)
        .map(|value| value.value)
}

#[test]
fn memory_that_reads_short_or_an_offset_past_the_address_space_is_unreadable() {
    assert_eq!(value("[0x20]+1"), Ok(8));
    assert_eq!(
        value("[0x10]"),
        Err(Fault::Unreadable(
            "cannot read 8 bytes at 0x0000000000000010".to_string()
        ))
    );
    assert_eq!(
        value("[0x20]-8"),
        Err(Fault::Unreadable(
            "'[0x0000000000000020]-0x8' is outside the address space".to_string()
        ))
    );
}

#[test]
fn a_vector_register_is_no_address() {
    assert_eq!(
        value("v0"),
        Err(Fault::Unknown(
            "'v0' is a vector register, not an address".to_string()
        ))
    );
}

#[test]
fn lt_compares_signed_and_in_holds_only_for_what_cannot_be_read() {
    let less = Condition::parse("x0", "lt", Some("0")).unwrap();
    assert_eq!(less.shown(), "x0 lt 0x0000000000000000 (0)");
    assert!(less.holds(&mut Thread, 0));
    assert!(!Condition::parse("x0", "lt", Some("-3"))
        .unwrap()
        .holds(&mut Thread, 0));
    let readable = Condition::parse("[0x20]", "in", None).unwrap();
    assert!(!readable.holds(&mut Thread, 0));
    let unreadable = Condition::parse("[0x30]", "in", None).unwrap();
    assert!(unreadable.holds(&mut Thread, 0));
}
