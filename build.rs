use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let general_dir = manifest_dir.join("src/man/builtins/general");
    let collection_dir = manifest_dir.join("src/man/builtins/collection");
    let filter_dir = manifest_dir.join("src/man/builtins/filter");
    let strings_dir = manifest_dir.join("src/man/builtins/strings");
    let types_dir = manifest_dir.join("src/man/types");
    let io_dir = manifest_dir.join("src/man/builtins/io");
    let math_dir = manifest_dir.join("src/man/builtins/math");
    let fs_dir = manifest_dir.join("src/man/builtins/fs");
    let thread_dir = manifest_dir.join("src/man/builtins/thread");
    let json_dir = manifest_dir.join("src/man/builtins/json");
    let types_page = manifest_dir.join("src/man/types/package.txt");
    let errors_page = manifest_dir.join("src/man/errors/package.txt");
    let unicode_page = manifest_dir.join("src/man/unicode/package.txt");
    println!("cargo:rerun-if-changed={}", general_dir.display());
    println!("cargo:rerun-if-changed={}", collection_dir.display());
    println!("cargo:rerun-if-changed={}", filter_dir.display());
    println!("cargo:rerun-if-changed={}", strings_dir.display());
    println!("cargo:rerun-if-changed={}", types_dir.display());
    println!("cargo:rerun-if-changed={}", io_dir.display());
    println!("cargo:rerun-if-changed={}", math_dir.display());
    println!("cargo:rerun-if-changed={}", fs_dir.display());
    println!("cargo:rerun-if-changed={}", thread_dir.display());
    println!("cargo:rerun-if-changed={}", json_dir.display());
    println!("cargo:rerun-if-changed={}", types_page.display());
    println!("cargo:rerun-if-changed={}", errors_page.display());
    println!("cargo:rerun-if-changed={}", unicode_page.display());

    let general_pages = man_pages(&general_dir, "general");
    let collection_pages = man_pages(&collection_dir, "collection");
    let filter_pages = man_pages(&filter_dir, "filter");
    let strings_pages = man_pages(&strings_dir, "strings");
    let types_pages = man_pages(&types_dir, "types");
    let io_pages = man_pages(&io_dir, "io");
    let math_pages = man_pages(&math_dir, "math");
    let fs_pages = man_pages(&fs_dir, "fs");
    let thread_pages = man_pages(&thread_dir, "thread");
    let json_pages = man_pages(&json_dir, "json");

    println!(
        "cargo:rerun-if-changed={}",
        general_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        collection_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        filter_dir.join("package.txt").display()
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
    for page in general_pages
        .iter()
        .chain(collection_pages.iter())
        .chain(filter_pages.iter())
        .chain(strings_pages.iter())
        .chain(types_pages.iter())
        .chain(io_pages.iter())
        .chain(math_pages.iter())
        .chain(fs_pages.iter())
        .chain(thread_pages.iter())
        .chain(json_pages.iter())
    {
        println!("cargo:rerun-if-changed={}", page.display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("man_generated.rs");
    let mut output = fs::File::create(out_path).expect("create generated man source");

    write_pages(&mut output, "GENERAL_FUNCTION_PAGES", general_pages);
    write_pages(&mut output, "COLLECTION_FUNCTION_PAGES", collection_pages);
    write_pages(&mut output, "FILTER_FUNCTION_PAGES", filter_pages);
    write_pages(&mut output, "STRINGS_FUNCTION_PAGES", strings_pages);
    write_pages(&mut output, "TYPES_TOPIC_PAGES", types_pages);
    write_pages(&mut output, "IO_FUNCTION_PAGES", io_pages);
    write_pages(&mut output, "MATH_FUNCTION_PAGES", math_pages);
    write_pages(&mut output, "FS_FUNCTION_PAGES", fs_pages);
    write_pages(&mut output, "THREAD_FUNCTION_PAGES", thread_pages);
    write_pages(&mut output, "JSON_FUNCTION_PAGES", json_pages);
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
