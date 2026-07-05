use std::env;
use std::io::IsTerminal;

use crate::docs::{render, spec};
use crate::USAGE;

/// `mfb spec [topic] [subtopic] [--width N] [--color|--no-color]`. Renders the
/// embedded Markdown specification to the terminal, reflowing to the terminal
/// width so tables stay readable.
pub(crate) fn show_spec(args: &[String]) -> Result<(), String> {
    let mut width: Option<usize> = None;
    let mut color: Option<bool> = None;
    let mut all = false;
    let mut positional: Vec<&str> = Vec::new();

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--all" => all = true,
            "--no-color" => color = Some(false),
            "--color" => color = Some(true),
            "--width" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "mfb spec --width requires a number".to_string())?;
                width = Some(parse_spec_width(value)?);
            }
            other if other.starts_with("--width=") => {
                width = Some(parse_spec_width(&other["--width=".len()..])?);
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option `{other}`\n\n{USAGE}"));
            }
            other => positional.push(other),
        }
    }

    let style = render::Style {
        width: width.unwrap_or_else(detect_terminal_width),
        color: color.unwrap_or_else(|| std::io::stdout().is_terminal()),
    };

    match positional.as_slice() {
        [] => {
            if all {
                return Err(format!("mfb spec --all requires a topic\n\n{USAGE}"));
            }
            print_spec_index(&style);
            Ok(())
        }
        [package_name] => {
            let package = spec::package(package_name)
                .ok_or_else(|| unknown_spec_package_error(package_name))?;
            if all {
                print_spec_all(package, &style);
            } else {
                print_spec_package(package, &style);
            }
            Ok(())
        }
        [package_name, topic_name] => {
            if all {
                return Err("mfb spec --all cannot be combined with a subtopic".to_string());
            }
            let package = spec::package(package_name)
                .ok_or_else(|| unknown_spec_package_error(package_name))?;
            let topic = spec::topic(package, topic_name).ok_or_else(|| {
                format!(
                    "unknown topic `{topic_name}` in spec `{package_name}`\n\nRun `mfb spec {package_name}` to list available topics."
                )
            })?;
            println!("{}", render::render(topic.page, &style));
            Ok(())
        }
        _ => Err(format!("mfb spec accepts at most two arguments\n\n{USAGE}")),
    }
}

fn parse_spec_width(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid --width value `{value}`"))
        .map(|width| width.clamp(20, 1000))
}

/// Terminal width for spec rendering. Prefer an explicit `COLUMNS` override,
/// then ask the terminal itself via `TIOCGWINSZ`, then fall back to the classic
/// 80 (also used when stdout is piped/redirected and has no window size).
///
/// Shared with `mfb man`, which renders Markdown man pages through the same
/// renderer and wants identical width behaviour.
pub(crate) fn detect_terminal_width() -> usize {
    if let Some(width) = env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
    {
        return width.clamp(20, 1000);
    }
    if let Some(width) = terminal_width_from_ioctl() {
        return width.clamp(20, 1000);
    }
    80
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn terminal_width_from_ioctl() -> Option<usize> {
    use std::os::raw::{c_int, c_ulong};

    #[repr(C)]
    struct Winsize {
        rows: u16,
        cols: u16,
        xpixel: u16,
        ypixel: u16,
    }

    extern "C" {
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    #[cfg(target_os = "macos")]
    const TIOCGWINSZ: c_ulong = 0x4008_7468;
    #[cfg(target_os = "linux")]
    const TIOCGWINSZ: c_ulong = 0x5413;

    let mut ws = Winsize {
        rows: 0,
        cols: 0,
        xpixel: 0,
        ypixel: 0,
    };
    // SAFETY: `ws` is a valid, properly aligned `winsize` that lives across the
    // call; `ioctl` only writes into it. Querying stdout (fd 1) on a non-tty
    // returns a non-zero status, which we treat as "unknown".
    let rc = unsafe { ioctl(1, TIOCGWINSZ, std::ptr::addr_of_mut!(ws)) };
    (rc == 0 && ws.cols > 0).then_some(ws.cols as usize)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn terminal_width_from_ioctl() -> Option<usize> {
    None
}

fn print_spec_index(style: &render::Style) {
    println!("Usage: mfb spec [topic] [subtopic] [--all]");
    println!();
    println!("Show the MFBASIC language specification.");
    println!();
    println!("Examples:");
    println!("  mfb spec");
    println!("  mfb spec architecture");
    println!("  mfb spec architecture native");
    println!("  mfb spec architecture --all");
    println!();
    println!("Topics:");
    println!();
    let entries: Vec<(&str, &str)> = spec::packages()
        .iter()
        .map(|package| (package.name, package.summary.as_str()))
        .collect();
    print_spec_listing("Topic", &entries, style);
}

fn print_spec_package(package: &spec::SpecPackage, style: &render::Style) {
    println!("{}", render::render(package.overview, style));
    if !package.topics.is_empty() {
        println!();
        let entries: Vec<(&str, &str)> = package
            .topics
            .iter()
            .map(|topic| (topic.name, topic.summary.as_str()))
            .collect();
        print_spec_listing("Subtopic", &entries, style);
        println!();
        println!("Run `mfb spec {} <subtopic>` for details.", package.name);
    }
}

/// `mfb spec <topic> --all`: print the overview followed by every subtopic page,
/// each separated by a full-width rule, as one continuous document.
fn print_spec_all(package: &spec::SpecPackage, style: &render::Style) {
    println!("{}", render::render(package.overview, style));
    for topic in &package.topics {
        println!();
        println!("{}", "─".repeat(style.width));
        println!();
        println!("{}", render::render(topic.page, style));
    }
}

/// Render a `(name, summary)` listing as a width-aware table through the spec
/// renderer, so the summary column wraps instead of running off the terminal.
fn print_spec_listing(heading: &str, entries: &[(&str, &str)], style: &render::Style) {
    if entries.is_empty() {
        return;
    }
    let mut markdown = format!("| {heading} | Summary |\n| --- | --- |\n");
    for (name, summary) in entries {
        markdown.push_str(&format!(
            "| {} | {} |\n",
            escape_spec_cell(name),
            escape_spec_cell(summary),
        ));
    }
    println!("{}", render::render(&markdown, style));
}

/// Escape a literal `|` so it stays inside its table cell rather than starting a
/// new column.
fn escape_spec_cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn unknown_spec_package_error(package_name: &str) -> String {
    let packages = spec::packages()
        .iter()
        .map(|package| package.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!("unknown spec topic `{package_name}`\n\nAvailable topics: {packages}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parse_spec_width_clamps_and_rejects_non_numbers() {
        assert_eq!(parse_spec_width("40"), Ok(40));
        // Below/above the clamp bounds are pinned to [20, 1000].
        assert_eq!(parse_spec_width("5"), Ok(20));
        assert_eq!(parse_spec_width("100000"), Ok(1000));
        let err = parse_spec_width("wide").unwrap_err();
        assert!(err.contains("invalid --width value `wide`"));
    }

    #[test]
    fn unknown_spec_package_error_lists_available_topics() {
        let err = unknown_spec_package_error("nope");
        assert!(err.contains("unknown spec topic `nope`"));
        assert!(err.contains("Available topics: "));
        // A real package name appears in the listing.
        assert!(err.contains("architecture"));
    }

    #[test]
    fn escape_spec_cell_escapes_pipes() {
        assert_eq!(escape_spec_cell("a|b|c"), "a\\|b\\|c");
        assert_eq!(escape_spec_cell("plain"), "plain");
    }

    #[test]
    fn detect_terminal_width_prefers_columns_env() {
        // Serialize on the process-global env by using a distinctive value and
        // restoring it. COLUMNS parses and clamps into [20, 1000].
        let previous = env::var("COLUMNS").ok();
        std::env::set_var("COLUMNS", "137");
        assert_eq!(detect_terminal_width(), 137);
        std::env::set_var("COLUMNS", "999999");
        assert_eq!(detect_terminal_width(), 1000);
        match previous {
            Some(value) => std::env::set_var("COLUMNS", value),
            None => std::env::remove_var("COLUMNS"),
        }
    }

    #[test]
    fn show_spec_index_succeeds_with_no_arguments() {
        assert!(show_spec(&s(&["--width", "80", "--no-color"])).is_ok());
    }

    #[test]
    fn show_spec_all_with_no_topic_is_an_error() {
        let err = show_spec(&s(&["--all"])).unwrap_err();
        assert!(err.contains("mfb spec --all requires a topic"));
    }

    #[test]
    fn show_spec_renders_a_known_package() {
        assert!(show_spec(&s(&["architecture", "--no-color", "--width=80"])).is_ok());
        // `--all` expands every subtopic page.
        assert!(show_spec(&s(&["architecture", "--all", "--no-color"])).is_ok());
    }

    #[test]
    fn show_spec_renders_a_known_subtopic() {
        assert!(show_spec(&s(&["architecture", "native", "--no-color"])).is_ok());
    }

    #[test]
    fn show_spec_rejects_unknown_package() {
        let err = show_spec(&s(&["definitely-not-a-topic"])).unwrap_err();
        assert!(err.contains("unknown spec topic"));
    }

    #[test]
    fn show_spec_rejects_unknown_subtopic() {
        let err = show_spec(&s(&["architecture", "definitely-not-a-subtopic"])).unwrap_err();
        assert!(err.contains("unknown topic"));
    }

    #[test]
    fn show_spec_rejects_all_with_subtopic() {
        let err = show_spec(&s(&["architecture", "native", "--all"])).unwrap_err();
        assert!(err.contains("--all cannot be combined with a subtopic"));
    }

    #[test]
    fn show_spec_rejects_too_many_positionals() {
        let err = show_spec(&s(&["a", "b", "c"])).unwrap_err();
        assert!(err.contains("at most two arguments"));
    }

    #[test]
    fn show_spec_rejects_unknown_option() {
        let err = show_spec(&s(&["--bogus"])).unwrap_err();
        assert!(err.contains("unknown option `--bogus`"));
    }

    #[test]
    fn show_spec_width_flag_requires_a_value() {
        let err = show_spec(&s(&["--width"])).unwrap_err();
        assert!(err.contains("--width requires a number"));
    }

    #[test]
    fn show_spec_accepts_color_flag() {
        assert!(show_spec(&s(&["--color", "architecture"])).is_ok());
    }
}
