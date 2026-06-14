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
        parse_types_package(),
        parse_errors_package(),
        parse_general_package(),
        parse_collection_package(),
        parse_filter_package(),
        parse_strings_package(),
        parse_unicode_package(),
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

    generated_pages(package.name)
        .and_then(|pages| pages.iter().find(|(name, _)| *name == local_name))
        .map(|(_, page)| *page)
}

fn parse_types_package() -> PackageDoc {
    let page = include_str!("types/package.txt");
    let (name, summary) = parse_name_line(page).expect("types package NAME line");

    PackageDoc {
        name,
        summary,
        usage: "mfb man types",
        functions: &[],
        page: Some(page),
    }
}

fn parse_errors_package() -> PackageDoc {
    let page = include_str!("errors/package.txt");
    let (name, summary) = parse_name_line(page).expect("errors package NAME line");

    PackageDoc {
        name,
        summary,
        usage: "mfb man errors",
        functions: &[],
        page: Some(page),
    }
}

fn parse_general_package() -> PackageDoc {
    let page = include_str!("builtins/general/package.txt");
    let (name, summary) = parse_name_line(page).expect("general package NAME line");
    let functions = generated::GENERAL_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man general [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_collection_package() -> PackageDoc {
    let page = include_str!("builtins/collection/package.txt");
    let (name, summary) = parse_name_line(page).expect("collection package NAME line");
    let functions = generated::COLLECTION_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man collection [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_filter_package() -> PackageDoc {
    let page = include_str!("builtins/filter/package.txt");
    let (name, summary) = parse_name_line(page).expect("filter package NAME line");
    let functions = generated::FILTER_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man filter [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_strings_package() -> PackageDoc {
    let page = include_str!("builtins/strings/package.txt");
    let (name, summary) = parse_name_line(page).expect("strings package NAME line");
    let functions = generated::STRINGS_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man strings [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_unicode_package() -> PackageDoc {
    let page = include_str!("unicode/package.txt");
    let (name, summary) = parse_name_line(page).expect("unicode package NAME line");

    PackageDoc {
        name,
        summary,
        usage: "mfb man unicode",
        functions: &[],
        page: Some(page),
    }
}

fn generated_pages(package_name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match package_name {
        "general" => Some(generated::GENERAL_FUNCTION_PAGES),
        "collection" => Some(generated::COLLECTION_FUNCTION_PAGES),
        "filter" => Some(generated::FILTER_FUNCTION_PAGES),
        "strings" => Some(generated::STRINGS_FUNCTION_PAGES),
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
        page: None,
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
