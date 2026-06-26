use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let general_dir = manifest_dir.join("src/man/builtins/general");
    let collections_dir = manifest_dir.join("src/man/builtins/collections");
    let filters_dir = manifest_dir.join("src/man/builtins/filters");
    let strings_dir = manifest_dir.join("src/man/builtins/strings");
    let types_dir = manifest_dir.join("src/man/types");
    let flow_dir = manifest_dir.join("src/man/flow");
    let io_dir = manifest_dir.join("src/man/builtins/io");
    let math_dir = manifest_dir.join("src/man/builtins/math");
    let fs_dir = manifest_dir.join("src/man/builtins/fs");
    let thread_dir = manifest_dir.join("src/man/builtins/thread");
    let json_dir = manifest_dir.join("src/man/builtins/json");
    let csv_dir = manifest_dir.join("src/man/builtins/csv");
    let regex_dir = manifest_dir.join("src/man/builtins/regex");
    let term_dir = manifest_dir.join("src/man/builtins/term");
    let datetime_dir = manifest_dir.join("src/man/builtins/datetime");
    let types_page = manifest_dir.join("src/man/types/package.txt");
    let errors_page = manifest_dir.join("src/man/errors/package.txt");
    let unicode_page = manifest_dir.join("src/man/unicode/package.txt");
    let lambda_page = manifest_dir.join("src/man/lambda/package.txt");
    let error_codes_doc = manifest_dir.join("specifications/error_codes.md");
    println!("cargo:rerun-if-changed={}", error_codes_doc.display());
    println!("cargo:rerun-if-changed={}", general_dir.display());
    println!("cargo:rerun-if-changed={}", collections_dir.display());
    println!("cargo:rerun-if-changed={}", filters_dir.display());
    println!("cargo:rerun-if-changed={}", strings_dir.display());
    println!("cargo:rerun-if-changed={}", types_dir.display());
    println!("cargo:rerun-if-changed={}", flow_dir.display());
    println!("cargo:rerun-if-changed={}", io_dir.display());
    println!("cargo:rerun-if-changed={}", math_dir.display());
    println!("cargo:rerun-if-changed={}", fs_dir.display());
    println!("cargo:rerun-if-changed={}", thread_dir.display());
    println!("cargo:rerun-if-changed={}", json_dir.display());
    println!("cargo:rerun-if-changed={}", csv_dir.display());
    println!("cargo:rerun-if-changed={}", regex_dir.display());
    println!("cargo:rerun-if-changed={}", term_dir.display());
    println!("cargo:rerun-if-changed={}", datetime_dir.display());
    println!("cargo:rerun-if-changed={}", types_page.display());
    println!("cargo:rerun-if-changed={}", errors_page.display());
    println!("cargo:rerun-if-changed={}", unicode_page.display());
    println!("cargo:rerun-if-changed={}", lambda_page.display());

    let general_pages = man_pages(&general_dir, "general");
    let collections_pages = man_pages(&collections_dir, "collections");
    let filters_pages = man_pages(&filters_dir, "filters");
    let strings_pages = man_pages(&strings_dir, "strings");
    let types_pages = man_pages(&types_dir, "types");
    let flow_pages = man_pages(&flow_dir, "flow");
    let io_pages = man_pages(&io_dir, "io");
    let math_pages = man_pages(&math_dir, "math");
    let fs_pages = man_pages(&fs_dir, "fs");
    let thread_pages = man_pages(&thread_dir, "thread");
    let json_pages = man_pages(&json_dir, "json");
    let csv_pages = man_pages(&csv_dir, "csv");
    let regex_pages = man_pages(&regex_dir, "regex");
    let term_pages = man_pages(&term_dir, "term");
    let datetime_pages = man_pages(&datetime_dir, "datetime");

    println!(
        "cargo:rerun-if-changed={}",
        general_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        filters_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        strings_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        types_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        flow_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        io_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        math_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        fs_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        thread_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        json_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        csv_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        regex_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        term_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        datetime_dir.join("package.txt").display()
    );
    for page in general_pages
        .iter()
        .chain(collections_pages.iter())
        .chain(filters_pages.iter())
        .chain(strings_pages.iter())
        .chain(types_pages.iter())
        .chain(flow_pages.iter())
        .chain(io_pages.iter())
        .chain(math_pages.iter())
        .chain(fs_pages.iter())
        .chain(thread_pages.iter())
        .chain(json_pages.iter())
        .chain(csv_pages.iter())
        .chain(regex_pages.iter())
        .chain(term_pages.iter())
        .chain(datetime_pages.iter())
    {
        println!("cargo:rerun-if-changed={}", page.display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("man_generated.rs");
    let mut output = fs::File::create(out_path).expect("create generated man source");

    write_pages(&mut output, "GENERAL_FUNCTION_PAGES", general_pages);
    write_pages(&mut output, "COLLECTIONS_FUNCTION_PAGES", collections_pages);
    write_pages(&mut output, "FILTERS_FUNCTION_PAGES", filters_pages);
    write_pages(&mut output, "STRINGS_FUNCTION_PAGES", strings_pages);
    write_pages(&mut output, "TYPES_TOPIC_PAGES", types_pages);
    write_pages(&mut output, "FLOW_TOPIC_PAGES", flow_pages);
    write_pages(&mut output, "IO_FUNCTION_PAGES", io_pages);
    write_pages(&mut output, "MATH_FUNCTION_PAGES", math_pages);
    write_pages(&mut output, "FS_FUNCTION_PAGES", fs_pages);
    write_pages(&mut output, "THREAD_FUNCTION_PAGES", thread_pages);
    write_pages(&mut output, "JSON_FUNCTION_PAGES", json_pages);
    write_pages(&mut output, "CSV_FUNCTION_PAGES", csv_pages);
    write_pages(&mut output, "REGEX_FUNCTION_PAGES", regex_pages);
    write_pages(&mut output, "TERM_FUNCTION_PAGES", term_pages);
    write_pages(&mut output, "DATETIME_FUNCTION_PAGES", datetime_pages);

    generate_errorcode_table(&error_codes_doc, &out_dir);
}

/// Parse the "Runtime and Standard Package Errors" table in
/// `specifications/error_codes.md` and emit a generated `(name, integer)`
/// table for the built-in `errorCode` package. The doc is the single source of
/// truth (plan-06-errorcodes.md §4a); this keeps the package from drifting from
/// the canonical registry. Only the runtime `Err*` rows are exported — those are
/// the program-visible `Error.code` values, matching `errorCode::Err*` usage.
fn generate_errorcode_table(doc_path: &PathBuf, out_dir: &PathBuf) {
    let doc = fs::read_to_string(doc_path).expect("read specifications/error_codes.md");

    let mut in_section = false;
    let mut rows: Vec<(String, String)> = Vec::new();
    for line in doc.lines() {
        if line.starts_with("## ") {
            // The runtime registry table lives under this one heading; any other
            // top-level heading ends it.
            in_section = line.contains("Runtime and Standard Package Errors");
            continue;
        }
        if !in_section || !line.trim_start().starts_with("| `") {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // | `code` | `integer` | `Name` | meaning | notes |  -> cells[1..4]
        if cells.len() < 4 {
            continue;
        }
        let code = cells[1].trim_matches('`');
        let integer = cells[2].trim_matches('`');
        let name = cells[3].trim_matches('`');
        if code.is_empty() || integer.is_empty() || name.is_empty() {
            continue;
        }
        // Defend against doc drift: hyphen-stripping the canonical code must equal
        // the integer column, and the integer must be a bare number.
        assert_eq!(
            code.replace('-', ""),
            integer,
            "error_codes.md row `{name}`: code `{code}` does not match integer `{integer}`",
        );
        assert!(
            integer.chars().all(|c| c.is_ascii_digit()),
            "error_codes.md row `{name}`: integer `{integer}` is not numeric",
        );
        rows.push((name.to_string(), integer.to_string()));
    }

    assert!(
        !rows.is_empty(),
        "no runtime error-code rows parsed from {}",
        doc_path.display()
    );

    let out_path = out_dir.join("errorcode_generated.rs");
    let mut output = fs::File::create(out_path).expect("create generated errorcode source");
    writeln!(
        output,
        "/// `(name, integer-literal)` for every runtime registry row, generated\n\
         /// from specifications/error_codes.md by build.rs. Do not edit by hand.\n\
         pub(crate) const ERRORCODE_CONSTANTS: &[(&str, &str)] = &["
    )
    .expect("write generated errorcode source");
    for (name, integer) in &rows {
        writeln!(output, "    ({name:?}, {integer:?}),").expect("write generated errorcode source");
    }
    writeln!(output, "];").expect("write generated errorcode source");
}

fn man_pages(dir: &PathBuf, package: &str) -> Vec<PathBuf> {
    let mut pages = fs::read_dir(dir)
        .unwrap_or_else(|_| panic!("read {package} man directory"))
        .map(|entry| entry.expect("read general man entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .filter(|path| path.file_name().is_some_and(|name| name != "package.txt"))
        .collect::<Vec<_>>();

    pages.sort();
    pages
}

fn write_pages(output: &mut fs::File, constant: &str, pages: Vec<PathBuf>) {
    writeln!(output, "pub(crate) const {constant}: &[(&str, &str)] = &[")
        .expect("write generated man source");

    for page in pages {
        let name = page
            .file_stem()
            .and_then(|name| name.to_str())
            .expect("general man page file stem");
        writeln!(
            output,
            "    ({name:?}, include_str!({path:?})),",
            path = page.display().to_string()
        )
        .expect("write generated man source");
    }

    writeln!(output, "];").expect("write generated man source");
}
