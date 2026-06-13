use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let general_dir = manifest_dir.join("src/man/builtins/general");
    let collection_dir = manifest_dir.join("src/man/builtins/collection");
    println!("cargo:rerun-if-changed={}", general_dir.display());
    println!("cargo:rerun-if-changed={}", collection_dir.display());

    let general_pages = man_pages(&general_dir, "general");
    let collection_pages = man_pages(&collection_dir, "collection");

    println!(
        "cargo:rerun-if-changed={}",
        general_dir.join("package.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        collection_dir.join("package.txt").display()
    );
    for page in general_pages.iter().chain(collection_pages.iter()) {
        println!("cargo:rerun-if-changed={}", page.display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("man_generated.rs");
    let mut output = fs::File::create(out_path).expect("create generated man source");

    write_pages(&mut output, "GENERAL_FUNCTION_PAGES", general_pages);
    write_pages(&mut output, "COLLECTION_FUNCTION_PAGES", collection_pages);
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
