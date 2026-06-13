use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let general_dir = manifest_dir.join("src/man/builtins/general");
    println!("cargo:rerun-if-changed={}", general_dir.display());

    let mut pages = fs::read_dir(&general_dir)
        .expect("read general man directory")
        .map(|entry| entry.expect("read general man entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .filter(|path| path.file_name().is_some_and(|name| name != "package.txt"))
        .collect::<Vec<_>>();

    pages.sort();
    println!(
        "cargo:rerun-if-changed={}",
        general_dir.join("package.txt").display()
    );
    for page in &pages {
        println!("cargo:rerun-if-changed={}", page.display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("man_generated.rs");
    let mut output = fs::File::create(out_path).expect("create generated man source");

    writeln!(
        output,
        "pub(crate) const GENERAL_FUNCTION_PAGES: &[(&str, &str)] = &["
    )
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
