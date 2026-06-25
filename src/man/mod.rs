use std::sync::LazyLock;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/man_generated.rs"));
}

pub(crate) struct PackageDoc {
    pub(crate) name: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) usage: &'static str,
    pub(crate) functions: &'static [FunctionDoc],
    pub(crate) page: Option<&'static str>,
}

pub(crate) struct FunctionDoc {
    pub(crate) name: &'static str,
    pub(crate) signature: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) example: &'static str,
}

static PACKAGES: LazyLock<Vec<PackageDoc>> = LazyLock::new(|| {
    vec![
        parse_package(include_str!("types/package.txt"), "mfb man types [topic]"),
        parse_package(include_str!("flow/package.txt"), "mfb man flow [topic]"),
        parse_package(include_str!("errors/package.txt"), "mfb man errors"),
        parse_package(
            include_str!("builtins/general/package.txt"),
            "mfb man general [function]",
        ),
        parse_package(
            include_str!("builtins/collections/package.txt"),
            "mfb man collections [function]",
        ),
        parse_package(
            include_str!("builtins/filters/package.txt"),
            "mfb man filters [function]",
        ),
        parse_package(
            include_str!("builtins/strings/package.txt"),
            "mfb man strings [function]",
        ),
        parse_package(include_str!("unicode/package.txt"), "mfb man unicode"),
        parse_package(include_str!("lambda/package.txt"), "mfb man lambda"),
        parse_package(
            include_str!("builtins/io/package.txt"),
            "mfb man io [function]",
        ),
        parse_package(
            include_str!("builtins/math/package.txt"),
            "mfb man math [function]",
        ),
        parse_package(
            include_str!("builtins/fs/package.txt"),
            "mfb man fs [function]",
        ),
        parse_package(
            include_str!("builtins/thread/package.txt"),
            "mfb man thread [function]",
        ),
        parse_package(
            include_str!("builtins/json/package.txt"),
            "mfb man json [function]",
        ),
        parse_package(
            include_str!("builtins/regex/package.txt"),
            "mfb man regex [function]",
        ),
        parse_package(
            include_str!("builtins/term/package.txt"),
            "mfb man term [function]",
        ),
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

    generated_pages(package.name)
        .and_then(|pages| pages.iter().find(|(name, _)| *name == local_name))
        .map(|(_, page)| *page)
}

fn parse_package(page: &'static str, usage: &'static str) -> PackageDoc {
    let (name, summary) = parse_name_line(page).expect("package NAME line");
    let functions = generated_pages(name)
        .map(|pages| {
            let docs = pages
                .iter()
                .map(|(_, page)| parse_rendered_function_page(page))
                .collect::<Vec<_>>()
                .into_boxed_slice();
            &*Box::leak(docs)
        })
        .unwrap_or(&[]);

    PackageDoc {
        name,
        summary,
        usage,
        functions,
        page: Some(page),
    }
}

fn generated_pages(package_name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match package_name {
        "types" => Some(generated::TYPES_TOPIC_PAGES),
        "flow" => Some(generated::FLOW_TOPIC_PAGES),
        "general" => Some(generated::GENERAL_FUNCTION_PAGES),
        "collections" => Some(generated::COLLECTIONS_FUNCTION_PAGES),
        "filters" => Some(generated::FILTERS_FUNCTION_PAGES),
        "strings" => Some(generated::STRINGS_FUNCTION_PAGES),
        "io" => Some(generated::IO_FUNCTION_PAGES),
        "math" => Some(generated::MATH_FUNCTION_PAGES),
        "fs" => Some(generated::FS_FUNCTION_PAGES),
        "thread" => Some(generated::THREAD_FUNCTION_PAGES),
        "json" => Some(generated::JSON_FUNCTION_PAGES),
        "regex" => Some(generated::REGEX_FUNCTION_PAGES),
        "term" => Some(generated::TERM_FUNCTION_PAGES),
        _ => None,
    }
}

fn parse_rendered_function_page(page: &'static str) -> FunctionDoc {
    let (name, summary) = parse_name_line(page).expect("function NAME line");
    FunctionDoc {
        name,
        signature: first_synopsis_line(page).unwrap_or(""),
        summary,
        example: "",
    }
}

fn parse_name_line(source: &'static str) -> Option<(&'static str, &'static str)> {
    let mut lines = source.lines();
    while !lines.next()?.trim().eq("NAME") {}

    let line = lines.find(|line| !line.trim().is_empty())?.trim();
    line.split_once(" - ")
}

fn first_synopsis_line(source: &'static str) -> Option<&'static str> {
    let mut lines = source.lines();
    while !lines.next()?.trim().eq("SYNOPSIS") {}

    lines
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .filter(|line| !line.is_empty())
}
