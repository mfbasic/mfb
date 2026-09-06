/// Every `raise_error("<function_id>", "<ErrName>")` literal site in
/// `src/codegen/` must name an error its owning descriptor declares — the
/// static, exhaustive form of `raise_error`'s `debug_assert`. That assert
/// only fires when a program actually lowers the member under a debug
/// binary, so an undeclared raise ships silently until someone compiles
/// the right call (`collections.findLastIndex` did exactly that: its
/// source-generic surface had no descriptor at all, and the fast path's
/// raises panicked the first debug-build user). This scan catches every
/// site at `cargo test` time, reachable or not.
#[test]
fn every_raise_error_site_is_declared_in_its_descriptor() {
    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read src/codegen") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/codegen");
    let mut files = Vec::new();
    rust_files(&root, &mut files);

    // `raise_error(` then two string literals, whitespace/newline tolerant
    // (rustfmt may split the arguments across lines). `self.`/`builder.`
    // receivers both match; `raise_error_bare` sites (no owning builtin)
    // deliberately do not. Parsed by hand — the crate carries no regex dep.
    fn quoted(text: &str, from: usize) -> Option<(String, usize)> {
        let rest = &text[from..];
        let open = from + rest.find('"')?;
        // Only skip whitespace/comma between the anchor and the literal —
        // anything else is not a literal-argument site.
        if !text[from..open]
            .chars()
            .all(|c| c.is_whitespace() || c == ',')
        {
            return None;
        }
        let close = open + 1 + text[open + 1..].find('"')?;
        Some((text[open + 1..close].to_string(), close + 1))
    }

    let registry = crate::codegen::registry::registry();
    let mut undeclared = Vec::new();
    let mut sites = 0usize;
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read source file");
        let mut at = 0usize;
        while let Some(hit) = text[at..].find("raise_error(") {
            let after = at + hit + "raise_error(".len();
            at = after;
            let Some((function_id, next)) = quoted(&text, after) else {
                continue;
            };
            let Some((error_name, _)) = quoted(&text, next) else {
                continue;
            };
            if !error_name.starts_with("Err") {
                continue;
            }
            let function_id = function_id.as_str();
            let error_name = error_name.as_str();
            sites += 1;
            let declared = registry.declares_error(&format!("general.{function_id}"), error_name)
                || registry.declares_error(function_id, error_name);
            if !declared {
                undeclared.push(format!(
                    "{}: {function_id} raises {error_name}",
                    file.strip_prefix(env!("CARGO_MANIFEST_DIR"))
                        .unwrap()
                        .display()
                ));
            }
        }
    }
    assert!(
        sites > 0,
        "the scan found no raise_error sites — the parser or root is wrong"
    );
    assert!(
        undeclared.is_empty(),
        "raise_error sites naming errors their descriptor does not declare \
             (add the error to the member's descriptor `errors`):\n{}",
        undeclared.join("\n")
    );
}
