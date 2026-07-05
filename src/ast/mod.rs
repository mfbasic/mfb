use crate::json_string;
use crate::lexer::{self, Keyword, Token, TokenKind};
use crate::rules;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

mod expr;
mod items;
mod lexical;
mod manifest;
mod parser;
mod serialize;
mod stmt;
mod types;

#[cfg(test)]
mod tests;

pub use items::normalize_ws;
pub use manifest::{
    parse_project, parse_source, parse_source_internal, selected_source_paths, write_ast,
    BUILTIN_PRELUDE_PATH,
};
pub use types::*;

use parser::{BlockTerminator, FileParser};
use serialize::{contains_placeholder, substitute_placeholder};

#[cfg(test)]
pub use manifest::augment_with_prelude;
#[cfg(test)]
use manifest::{collect_selected_source_files, glob_matches, SelectedSource};
