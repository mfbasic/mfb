use std::io::IsTerminal;

use crate::cli::spec::detect_terminal_width;
use crate::docs::man;
use crate::docs::render;
use crate::USAGE;

pub(crate) fn show_man(args: &[String]) -> Result<(), String> {
    match args {
        [] => {
            print_man_index();
            Ok(())
        }
        [package_name] => {
            let package =
                man::package(package_name).ok_or_else(|| unknown_package_error(package_name))?;
            print_package_man(package);
            Ok(())
        }
        [package_name, function_name] => {
            let package =
                man::package(package_name).ok_or_else(|| unknown_package_error(package_name))?;
            let function = man::function(package, function_name).ok_or_else(|| {
                format!(
                    "unknown function `{function_name}` in package `{package_name}`\n\nRun `mfb man {package_name}` to list available functions."
                )
            })?;
            if let Some(page) = man::function_page(package, function_name) {
                print_man_page(page);
            } else {
                print_function_man(package, function);
            }
            Ok(())
        }
        _ => Err(format!("mfb man accepts at most two arguments\n\n{USAGE}")),
    }
}

fn print_man_index() {
    println!("Usage: mfb man [package] [function]");
    println!();
    println!("Show help for built-in packages and functions.");
    println!();
    println!("Examples:");
    println!("  mfb man");
    println!("  mfb man general");
    println!("  mfb man io print");
    println!();
    println!("Packages:");
    for package in man::packages() {
        println!("  {:<8} {}", package.name, package.summary);
    }
}

fn print_package_man(package: &man::PackageDoc) {
    if let Some(page) = package.page {
        print_man_page(page);
        if !package.functions.is_empty() {
            println!();
            print_entry_listing(package, false);
        }
        return;
    }

    println!("Package: {}", package.name);
    println!();
    println!("{}", package.summary);
    println!();
    println!("Usage:");
    println!("  {}", package.usage);
    println!();
    print_entry_listing(package, true);
}

/// Print the package's entries, splitting constants out from functions so a
/// reference such as `math::pi` is never listed alongside callables like
/// `math::sin`. `colon_heading` matches the legacy `FUNCTIONS:` styling of the
/// pageless layout. The trailing hint always points at the entry kind the
/// package's two-argument lookup expects.
fn print_entry_listing(package: &man::PackageDoc, colon_heading: bool) {
    let (constants, functions): (Vec<_>, Vec<_>) = package
        .functions
        .iter()
        .partition(|function| is_constant(function));

    let colon = if colon_heading { ":" } else { "" };
    let mut printed = false;
    if !constants.is_empty() {
        println!("CONSTANTS{colon}");
        for constant in &constants {
            println!("  {:<18} {}", constant.name, constant.summary);
        }
        printed = true;
    }
    if !functions.is_empty() {
        if printed {
            println!();
        }
        println!("{}{colon}", man_entry_heading(package));
        for function in &functions {
            println!("  {:<18} {}", function.name, function.summary);
        }
    }

    println!();
    println!(
        "Run `mfb man {} <{}>` for details.",
        package.name,
        man_entry_name(package)
    );
}

/// A constant entry renders as a value reference (`math::pi AS Float`) rather
/// than a call: its synopsis carries the `package::name` qualifier and an
/// `AS <Type>` clause but no argument list. This deliberately excludes the
/// `flow`/`types` topic pages (no `::`) and the `json::types` record-type page
/// (no `AS`), leaving them under their usual heading.
fn is_constant(function: &man::FunctionDoc) -> bool {
    let signature = function.signature;
    !signature.contains('(') && signature.contains("::") && signature.contains(" AS ")
}

fn man_entry_heading(package: &man::PackageDoc) -> &'static str {
    if package.name == "types" {
        "TOPICS"
    } else {
        "FUNCTIONS"
    }
}

fn man_entry_name(package: &man::PackageDoc) -> &'static str {
    if package.name == "types" {
        "topic"
    } else {
        "function"
    }
}

/// Print a stored man page. Markdown pages go through the same width-aware
/// renderer as `mfb spec`; legacy plain-text pages are printed verbatim.
fn print_man_page(page: &str) {
    if man::is_markdown_page(page) {
        let style = render::Style {
            width: detect_terminal_width(),
            color: std::io::stdout().is_terminal(),
        };
        println!("{}", render::render(page, &style));
    } else {
        println!("{}", page.trim_end_matches('\n'));
    }
}

fn print_function_man(package: &man::PackageDoc, function: &man::FunctionDoc) {
    println!("{} {}", package.name, function.name);
    println!();
    println!("{}", function.summary);
    println!();
    println!("Signature:");
    println!("  {}", function.signature);
    println!();
    println!("Example:");
    for line in function.example.lines() {
        println!("  {line}");
    }
}

fn unknown_package_error(package_name: &str) -> String {
    let packages = man::packages()
        .iter()
        .map(|package| package.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!("unknown package `{package_name}`\n\nAvailable packages: {packages}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn show_man_index_with_no_arguments() {
        assert!(show_man(&s(&[])).is_ok());
    }

    #[test]
    fn show_man_renders_a_known_package() {
        // `io` has a page and function listing; `math` carries constants.
        assert!(show_man(&s(&["io"])).is_ok());
        assert!(show_man(&s(&["math"])).is_ok());
        assert!(show_man(&s(&["types"])).is_ok());
    }

    #[test]
    fn show_man_renders_a_known_function() {
        assert!(show_man(&s(&["io", "print"])).is_ok());
        // A constant reference (math::pi renders as a value, not a call).
        assert!(show_man(&s(&["math", "pi"])).is_ok());
    }

    #[test]
    fn show_man_rejects_unknown_package() {
        let err = show_man(&s(&["definitely-not-a-package"])).unwrap_err();
        assert!(err.contains("unknown package"));
        assert!(err.contains("Available packages:"));
    }

    #[test]
    fn show_man_rejects_unknown_function() {
        let err = show_man(&s(&["io", "definitely-not-a-fn"])).unwrap_err();
        assert!(err.contains("unknown function"));
    }

    #[test]
    fn show_man_rejects_too_many_arguments() {
        let err = show_man(&s(&["io", "print", "extra"])).unwrap_err();
        assert!(err.contains("at most two arguments"));
    }

    #[test]
    fn unknown_package_error_lists_packages() {
        let err = unknown_package_error("zzz");
        assert!(err.contains("unknown package `zzz`"));
        assert!(err.contains("io"));
    }

    #[test]
    fn is_constant_matches_qualified_typed_value_references() {
        let constant = man::FunctionDoc {
            name: "pi",
            summary: "circle constant",
            signature: "math::pi AS Float",
            example: "x = math::pi\n",
        };
        assert!(is_constant(&constant));
        let call = man::FunctionDoc {
            name: "sin",
            summary: "sine",
            signature: "math::sin(value AS Float) AS Float",
            example: "x = math::sin(0.0)\n",
        };
        // A call (has parens) is not a constant.
        assert!(!is_constant(&call));
        // A topic page without `::` is not a constant.
        let topic = man::FunctionDoc {
            name: "flow",
            summary: "flow",
            signature: "IF ... THEN",
            example: "",
        };
        assert!(!is_constant(&topic));
    }

    #[test]
    fn man_entry_heading_and_name_special_case_types() {
        let types = man::PackageDoc {
            name: "types",
            summary: "",
            usage: "",
            page: None,
            functions: &[],
        };
        assert_eq!(man_entry_heading(&types), "TOPICS");
        assert_eq!(man_entry_name(&types), "topic");
        let io = man::PackageDoc {
            name: "io",
            summary: "",
            usage: "",
            page: None,
            functions: &[],
        };
        assert_eq!(man_entry_heading(&io), "FUNCTIONS");
        assert_eq!(man_entry_name(&io), "function");
    }

    #[test]
    fn print_function_man_renders_a_pageless_function() {
        // Exercise the pageless function path directly (a package whose
        // function has no dedicated page).
        let function = man::FunctionDoc {
            name: "example",
            summary: "does a thing",
            signature: "pkg::example(x AS Integer) AS Integer",
            example: "line one\nline two\n",
        };
        let package = man::PackageDoc {
            name: "pkg",
            summary: "",
            usage: "",
            page: None,
            functions: &[],
        };
        // Should not panic while rendering.
        print_function_man(&package, &function);
    }

    #[test]
    fn print_man_page_handles_markdown_and_plain() {
        // Markdown page goes through the renderer; plain text is printed verbatim.
        print_man_page("# Heading\n\nbody text\n");
        print_man_page("plain legacy page\n");
    }

    #[test]
    fn print_entry_listing_splits_constants_and_functions() {
        static FUNCTIONS: &[man::FunctionDoc] = &[
            man::FunctionDoc {
                name: "pi",
                summary: "constant",
                signature: "math::pi AS Float",
                example: "",
            },
            man::FunctionDoc {
                name: "sin",
                summary: "sine",
                signature: "math::sin(x AS Float) AS Float",
                example: "",
            },
        ];
        let package = man::PackageDoc {
            name: "math",
            summary: "math",
            usage: "IMPORT math",
            page: None,
            functions: FUNCTIONS,
        };
        // Covers both the colon-heading and no-heading styles.
        print_entry_listing(&package, true);
        print_entry_listing(&package, false);
        // And the top-level pageless package printer.
        print_package_man(&package);
    }
}
