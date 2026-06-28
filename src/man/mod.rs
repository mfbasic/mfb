use std::sync::LazyLock;

// The generated `MAN_PACKAGES` table is a nested tuple slice by nature.
#[allow(clippy::type_complexity)]
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

/// Display order and usage synopsis for `mfb man`, in the order packages are
/// listed. The page content is generated from the `src/man` directory tree by
/// build.rs (`generated::MAN_PACKAGES`); this table carries only the editorial
/// bits the filesystem can't express. Dropping a `.txt` file adds a topic with
/// no edit here; a brand-new package needs one row.
const PACKAGE_ORDER: &[(&str, &str)] = &[
    ("types", "mfb man types [topic]"),
    ("flow", "mfb man flow [topic]"),
    ("errors", "mfb man errors"),
    ("general", "mfb man general [function]"),
    ("collections", "mfb man collections [function]"),
    ("filters", "mfb man filters [function]"),
    ("strings", "mfb man strings [function]"),
    ("unicode", "mfb man unicode"),
    ("lambda", "mfb man lambda"),
    ("io", "mfb man io [function]"),
    ("math", "mfb man math [function]"),
    ("bits", "mfb man bits [function]"),
    ("encoding", "mfb man encoding [function]"),
    ("fs", "mfb man fs [function]"),
    ("thread", "mfb man thread [function]"),
    ("json", "mfb man json [function]"),
    ("csv", "mfb man csv [function]"),
    ("regex", "mfb man regex [function]"),
    ("term", "mfb man term [function]"),
    ("datetime", "mfb man datetime [function]"),
    ("net", "mfb man net [function]"),
    ("tls", "mfb man tls [function]"),
    ("http", "mfb man http [function]"),
    ("vector", "mfb man vector [function]"),
];

static PACKAGES: LazyLock<Vec<PackageDoc>> = LazyLock::new(|| {
    debug_assert_eq!(
        PACKAGE_ORDER.len(),
        generated::MAN_PACKAGES.len(),
        "PACKAGE_ORDER is out of sync with the generated man packages",
    );
    PACKAGE_ORDER
        .iter()
        .map(|&(name, usage)| {
            let page = package_page(name)
                .unwrap_or_else(|| panic!("man package `{name}` missing generated docs"));
            build_package(name, usage, page)
        })
        .collect()
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
        .iter()
        .find(|(name, _)| *name == local_name)
        .map(|(_, page)| *page)
}

fn build_package(
    name: &'static str,
    usage: &'static str,
    page: &'static str,
) -> PackageDoc {
    let (parsed_name, summary) = parse_name_line(page).expect("package NAME line");
    debug_assert_eq!(
        parsed_name, name,
        "man package directory name disagrees with its NAME line",
    );
    let functions = generated_pages(name)
        .iter()
        .map(|(_, page)| parse_rendered_function_page(page))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    PackageDoc {
        name,
        summary,
        usage,
        functions: Box::leak(functions),
        page: Some(page),
    }
}

/// The `package.txt` overview for `package_name`, or `None` if no such package
/// was generated from the `src/man` tree.
fn package_page(package_name: &str) -> Option<&'static str> {
    generated::MAN_PACKAGES
        .iter()
        .find(|(name, _, _)| *name == package_name)
        .map(|(_, page, _)| *page)
}

/// The topic/function pages for `package_name`, or an empty slice when the
/// package has no pages (or does not exist).
fn generated_pages(package_name: &str) -> &'static [(&'static str, &'static str)] {
    generated::MAN_PACKAGES
        .iter()
        .find(|(name, _, _)| *name == package_name)
        .map(|(_, _, pages)| *pages)
        .unwrap_or(&[])
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
