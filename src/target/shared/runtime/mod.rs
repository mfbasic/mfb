use crate::builtins;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeHelper {
    Datetime,
    Fs,
    General,
    Io,
    Math,
    Net,
    Strings,
    Term,
    Thread,
    Tls,
}

impl RuntimeHelper {
    pub fn name(self) -> &'static str {
        match self {
            RuntimeHelper::Datetime => "datetime",
            RuntimeHelper::Fs => "fs",
            RuntimeHelper::General => "general",
            RuntimeHelper::Io => "io",
            RuntimeHelper::Math => "math",
            RuntimeHelper::Net => "net",
            RuntimeHelper::Strings => "strings",
            RuntimeHelper::Term => "term",
            RuntimeHelper::Thread => "thread",
            RuntimeHelper::Tls => "tls",
        }
    }
}

pub fn symbol_for_call(helper: RuntimeHelper, target: &str) -> String {
    format!(
        "_mfb_rt_{}_{}",
        helper.name(),
        target
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>()
    )
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeHelperSpec {
    pub(crate) helper: RuntimeHelper,
    pub(crate) call: &'static str,
    pub(crate) symbol: &'static str,
    pub(crate) abi: RuntimeHelperAbi,
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeHelperAbi {
    pub(crate) params: &'static [RuntimeAbiParam],
    pub(crate) returns: &'static str,
    pub(crate) clobbers: &'static [&'static str],
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeAbiParam {
    pub(crate) name: &'static str,
    pub(crate) type_: &'static str,
    pub(crate) location: &'static str,
}


mod catalog;
mod datetime_specs;
mod fs_specs;
mod io_specs;
mod net_specs;
mod strings_specs;
mod term_specs;
mod thread_specs;
mod usage;

pub(crate) use catalog::{spec_for_call, spec_for_symbol, supported_helper_specs};
pub(crate) use usage::{is_native_direct_call, required_helpers};

use datetime_specs::*;
use fs_specs::*;
use io_specs::*;
use net_specs::*;
use strings_specs::*;
use term_specs::*;
use thread_specs::*;

pub fn helper_for_call(name: &str) -> Option<RuntimeHelper> {
    if matches!(
        name,
        "datetime.nowNanos" | "datetime.monotonicNanos" | "datetime.localOffset"
    ) {
        Some(RuntimeHelper::Datetime)
    } else if builtins::fs::is_fs_call(name) {
        Some(RuntimeHelper::Fs)
    } else if builtins::general::is_general_call(name) {
        Some(RuntimeHelper::General)
    } else if builtins::io::is_io_call(name) {
        Some(RuntimeHelper::Io)
    } else if builtins::math::is_math_call(name) {
        Some(RuntimeHelper::Math)
    } else if builtins::strings::is_strings_call(name) {
        Some(RuntimeHelper::Strings)
    } else if builtins::term::is_term_call(name) {
        Some(RuntimeHelper::Term)
    } else if builtins::thread::is_thread_call(name) {
        Some(RuntimeHelper::Thread)
    } else if builtins::net::is_net_call(name) {
        Some(RuntimeHelper::Net)
    } else if builtins::tls::is_tls_call(name) {
        Some(RuntimeHelper::Tls)
    } else {
        None
    }
}
