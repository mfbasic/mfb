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
        parse_flow_package(),
        parse_errors_package(),
        parse_general_package(),
        parse_collections_package(),
        parse_filter_package(),
        parse_strings_package(),
        parse_unicode_package(),
        parse_io_package(),
        parse_math_package(),
        parse_fs_package(),
        parse_thread_package(),
        parse_json_package(),
        parse_regex_package(),
        parse_term_package(),
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
    let functions = generated::TYPES_TOPIC_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man types [topic]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_flow_package() -> PackageDoc {
    let page = include_str!("flow/package.txt");
    let (name, summary) = parse_name_line(page).expect("flow package NAME line");
    let functions = generated::FLOW_TOPIC_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man flow [topic]",
        functions: Box::leak(functions),
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

fn parse_collections_package() -> PackageDoc {
    let page = include_str!("builtins/collections/package.txt");
    let (name, summary) = parse_name_line(page).expect("collections package NAME line");
    let functions = generated::COLLECTIONS_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man collections [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_filter_package() -> PackageDoc {
    let page = include_str!("builtins/filters/package.txt");
    let (name, summary) = parse_name_line(page).expect("filters package NAME line");
    let functions = generated::FILTERS_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man filters [function]",
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

fn parse_io_package() -> PackageDoc {
    let page = include_str!("builtins/io/package.txt");
    let (name, summary) = parse_name_line(page).expect("io package NAME line");
    let functions = generated::IO_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man io [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_math_package() -> PackageDoc {
    let page = include_str!("builtins/math/package.txt");
    let (name, summary) = parse_name_line(page).expect("math package NAME line");
    let functions = generated::MATH_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man math [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_fs_package() -> PackageDoc {
    let page = include_str!("builtins/fs/package.txt");
    let (name, summary) = parse_name_line(page).expect("fs package NAME line");
    let functions = generated::FS_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man fs [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_thread_package() -> PackageDoc {
    let page = include_str!("builtins/thread/package.txt");
    let (name, summary) = parse_name_line(page).expect("thread package NAME line");
    let functions = generated::THREAD_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man thread [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_json_package() -> PackageDoc {
    let page = include_str!("builtins/json/package.txt");
    let (name, summary) = parse_name_line(page).expect("json package NAME line");
    let functions = generated::JSON_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man json [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_regex_package() -> PackageDoc {
    let page = include_str!("builtins/regex/package.txt");
    let (name, summary) = parse_name_line(page).expect("regex package NAME line");
    let functions = generated::REGEX_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man regex [function]",
        functions: Box::leak(functions),
        page: Some(page),
    }
}

fn parse_term_package() -> PackageDoc {
    let page = include_str!("builtins/term/package.txt");
    let (name, summary) = parse_name_line(page).expect("term package NAME line");
    let functions = generated::TERM_FUNCTION_PAGES
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage: "mfb man term [function]",
        functions: Box::leak(functions),
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
