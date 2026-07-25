use crate::json::json_string;
use crate::lexer::{self, Keyword, Token, TokenKind};
use crate::rules;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

pub(crate) mod build;
mod doc_items;
mod expr;
mod items;
mod lexical;
mod link_items;
pub(crate) mod manifest;
mod overloads;
mod parser;
mod pipeline;
pub(crate) mod scope_privates;
mod serialize;
mod stmt;
mod testing;
mod types;

#[cfg(test)]
mod tests;

pub use overloads::{normalize_types, normalize_ws, param_types};
pub use manifest::{
    parse_project, parse_source, parse_source_internal, selected_source_paths, write_ast,
    BUILTIN_PRELUDE_PATH,
};
pub use types::*;

pub use parser::SYNTHETIC_TRAP_BINDING;
use parser::{BlockTerminator, FileParser};

#[cfg(test)]
pub use manifest::augment_with_prelude;
#[cfg(test)]
use manifest::{collect_selected_source_files, glob_matches, SelectedSource};
