use std::sync::LazyLock;

pub(crate) struct PackageDoc {
    pub(crate) name: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) usage: &'static str,
    pub(crate) functions: &'static [FunctionDoc],
}

pub(crate) struct FunctionDoc {
    pub(crate) name: &'static str,
    pub(crate) signature: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) example: &'static str,
}

static PACKAGES: LazyLock<Vec<PackageDoc>> = LazyLock::new(|| {
    vec![
        parse_package(include_str!("builtins/general.txt")),
        parse_package(include_str!("builtins/io.txt")),
    ]
});

pub(crate) fn packages() -> &'static [PackageDoc] {
    PACKAGES.as_slice()
}

pub(crate) fn package(name: &str) -> Option<&'static PackageDoc> {
    packages().iter().find(|package| package.name == name)
}

pub(crate) fn function(package: &PackageDoc, name: &str) -> Option<&'static FunctionDoc> {
    let local_name = name
        .strip_prefix(package.name)
        .and_then(|remaining| remaining.strip_prefix('.'))
        .unwrap_or(name);
    package
        .functions
        .iter()
        .find(|function| function.name == local_name)
}

fn parse_package(source: &'static str) -> PackageDoc {
    let mut sections = source.split("\n---\n");
    let header = sections.next().expect("man page package header");
    let mut header_lines = header.lines();

    let name = parse_prefixed_line(&mut header_lines, "package:");
    let summary = parse_prefixed_line(&mut header_lines, "summary:");
    let usage = parse_prefixed_line(&mut header_lines, "usage:");

    let functions = sections
        .filter(|section| !section.trim().is_empty())
        .map(parse_function)
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage,
        functions: Box::leak(functions),
    }
}

fn parse_function(section: &'static str) -> FunctionDoc {
    let mut lines = section.lines();
    let name = parse_prefixed_line(&mut lines, "function:");
    let signature = parse_prefixed_line(&mut lines, "signature:");
    let summary = parse_prefixed_line(&mut lines, "summary:");

    let example_marker = "example:\n";
    let example_start = section
        .find(example_marker)
        .map(|index| index + example_marker.len())
        .expect("man page function example");

    FunctionDoc {
        name,
        signature,
        summary,
        example: section[example_start..].trim_end_matches('\n'),
    }
}

fn parse_prefixed_line(lines: &mut std::str::Lines<'static>, prefix: &'static str) -> &'static str {
    let line = lines.next().expect("man page line");
    line.strip_prefix(prefix)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("man page prefixed value")
}
