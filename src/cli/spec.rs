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
                return Err(
                    "mfb spec --all cannot be combined with a subtopic".to_string()
                );
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
