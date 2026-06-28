use crate::man;
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

fn print_man_page(page: &str) {
    println!("{}", page.trim_end_matches('\n'));
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
