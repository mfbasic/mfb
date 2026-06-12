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
        parse_package(include_str!("builtins/math.txt")),
        parse_package(include_str!("builtins/thread.txt")),
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

pub(crate) fn function_page(package: &PackageDoc, name: &str) -> Option<&'static str> {
    let local_name = name
        .strip_prefix(package.name)
        .and_then(|remaining| remaining.strip_prefix('.'))
        .unwrap_or(name);

    match (package.name, local_name) {
        ("general", "append") => Some(include_str!("builtins/general/append.txt")),
        ("general", "contains") => Some(include_str!("builtins/general/contains.txt")),
        ("general", "filter") => Some(include_str!("builtins/general/filter.txt")),
        ("general", "find") => Some(include_str!("builtins/general/find.txt")),
        ("general", "forEach") => Some(include_str!("builtins/general/forEach.txt")),
        ("general", "get") => Some(include_str!("builtins/general/get.txt")),
        ("general", "getOr") => Some(include_str!("builtins/general/getOr.txt")),
        ("general", "hasKey") => Some(include_str!("builtins/general/hasKey.txt")),
        ("general", "insert") => Some(include_str!("builtins/general/insert.txt")),
        ("general", "isEmpty") => Some(include_str!("builtins/general/isEmpty.txt")),
        ("general", "isEven") => Some(include_str!("builtins/general/isEven.txt")),
        ("general", "isNegative") => Some(include_str!("builtins/general/isNegative.txt")),
        ("general", "isNotEmpty") => Some(include_str!("builtins/general/isNotEmpty.txt")),
        ("general", "isNumeric") => Some(include_str!("builtins/general/isNumeric.txt")),
        ("general", "isOdd") => Some(include_str!("builtins/general/isOdd.txt")),
        ("general", "isPositive") => Some(include_str!("builtins/general/isPositive.txt")),
        ("general", "isZero") => Some(include_str!("builtins/general/isZero.txt")),
        ("general", "keys") => Some(include_str!("builtins/general/keys.txt")),
        ("general", "len") => Some(include_str!("builtins/general/len.txt")),
        ("general", "mid") => Some(include_str!("builtins/general/mid.txt")),
        ("general", "prepend") => Some(include_str!("builtins/general/prepend.txt")),
        ("general", "reduce") => Some(include_str!("builtins/general/reduce.txt")),
        ("general", "removeAt") => Some(include_str!("builtins/general/removeAt.txt")),
        ("general", "removeKey") => Some(include_str!("builtins/general/removeKey.txt")),
        ("general", "replace") => Some(include_str!("builtins/general/replace.txt")),
        ("general", "set") => Some(include_str!("builtins/general/set.txt")),
        ("general", "sum") => Some(include_str!("builtins/general/sum.txt")),
        ("general", "toByte") => Some(include_str!("builtins/general/toByte.txt")),
        ("general", "toFixed") => Some(include_str!("builtins/general/toFixed.txt")),
        ("general", "toFloat") => Some(include_str!("builtins/general/toFloat.txt")),
        ("general", "toInt") => Some(include_str!("builtins/general/toInt.txt")),
        ("general", "toString") => Some(include_str!("builtins/general/toString.txt")),
        ("general", "transform") => Some(include_str!("builtins/general/transform.txt")),
        ("general", "typeName") => Some(include_str!("builtins/general/typeName.txt")),
        ("general", "values") => Some(include_str!("builtins/general/values.txt")),
        _ => None,
    }
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
