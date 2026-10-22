use std::env;
use std::io::{IsTerminal, Write};

use crate::cli::help::USAGE;
use crate::docs::{render, spec};

/// `mfb spec [topic] [subtopic] [--width N] [--color|--no-color]`. Renders the
/// embedded Markdown specification to the terminal, reflowing to the terminal
/// width so tables stay readable.
///
/// A closed stdout (`mfb spec architecture --all | head`) is a normal way to stop
/// reading, so a broken pipe ends the output quietly; any other write failure is
/// reported and exits 1, matching `mfb man`.
pub(crate) fn show_spec(args: &[String]) -> Result<(), String> {
    let (request, style) = parse_spec_request(args)?;
    let stdout = std::io::stdout();
    if let Err(err) = write_spec(&mut stdout.lock(), &request, &style) {
        if err.kind() != std::io::ErrorKind::BrokenPipe {
            eprintln!("error: failed writing spec output: {err}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// What one `mfb spec` invocation prints, resolved before any output is written.
enum SpecRequest {
    Index,
    Package(&'static spec::SpecPackage),
    All(&'static spec::SpecPackage),
    Topic(&'static str),
}

fn parse_spec_request(args: &[String]) -> Result<(SpecRequest, render::Style), String> {
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

    let request = match positional.as_slice() {
        [] => {
            if all {
                return Err(format!("mfb spec --all requires a topic\n\n{USAGE}"));
            }
            SpecRequest::Index
        }
        [package_name] => {
            let package = spec::package(package_name)
                .ok_or_else(|| unknown_spec_package_error(package_name))?;
            if all {
                SpecRequest::All(package)
            } else {
                SpecRequest::Package(package)
            }
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
            SpecRequest::Topic(topic.page)
        }
        _ => return Err(format!("mfb spec accepts at most two arguments\n\n{USAGE}")),
    };
    Ok((request, style))
}

/// Write one resolved request, stopping at the first failed write.
fn write_spec(
    out: &mut impl Write,
    request: &SpecRequest,
    style: &render::Style,
) -> std::io::Result<()> {
    match request {
        SpecRequest::Index => print_spec_index(out, style),
        SpecRequest::Package(package) => print_spec_package(out, package, style),
        SpecRequest::All(package) => print_spec_all(out, package, style),
        SpecRequest::Topic(page) => writeln!(out, "{}", render::render(page, style)),
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

fn print_spec_index(out: &mut impl Write, style: &render::Style) -> std::io::Result<()> {
    writeln!(out, "Usage: mfb spec [topic] [subtopic] [--all]")?;
    writeln!(out)?;
    writeln!(out, "Show the MFBASIC language specification.")?;
    writeln!(out)?;
    writeln!(out, "Examples:")?;
    writeln!(out, "  mfb spec")?;
    writeln!(out, "  mfb spec architecture")?;
    writeln!(out, "  mfb spec architecture native")?;
    writeln!(out, "  mfb spec architecture --all")?;
    writeln!(out)?;
    writeln!(out, "Topics:")?;
    writeln!(out)?;
    let entries: Vec<(&str, &str)> = spec::packages()
        .iter()
        .map(|package| (package.name, package.summary.as_str()))
        .collect();
    print_spec_listing(out, "Topic", &entries, style)
}

fn print_spec_package(
    out: &mut impl Write,
    package: &spec::SpecPackage,
    style: &render::Style,
) -> std::io::Result<()> {
    writeln!(out, "{}", render::render(package.overview, style))?;
    if !package.topics.is_empty() {
        writeln!(out)?;
        let entries: Vec<(&str, &str)> = package
            .topics
            .iter()
            .map(|topic| (topic.name, topic.summary.as_str()))
            .collect();
        print_spec_listing(out, "Subtopic", &entries, style)?;
        writeln!(out)?;
        writeln!(
            out,
            "Run `mfb spec {} <subtopic>` for details.",
            package.name
        )?;
    }
    Ok(())
}

/// `mfb spec <topic> --all`: print the overview followed by every subtopic page,
/// each separated by a full-width rule, as one continuous document.
fn print_spec_all(
    out: &mut impl Write,
    package: &spec::SpecPackage,
    style: &render::Style,
) -> std::io::Result<()> {
    writeln!(out, "{}", render::render(package.overview, style))?;
    for topic in &package.topics {
        writeln!(out)?;
        writeln!(out, "{}", "─".repeat(style.width))?;
        writeln!(out)?;
        writeln!(out, "{}", render::render(topic.page, style))?;
    }
    Ok(())
}

/// Render a `(name, summary)` listing as a width-aware table through the spec
/// renderer, so the summary column wraps instead of running off the terminal.
fn print_spec_listing(
    out: &mut impl Write,
    heading: &str,
    entries: &[(&str, &str)],
    style: &render::Style,
) -> std::io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut markdown = format!("| {heading} | Summary |\n| --- | --- |\n");
    for (name, summary) in entries {
        markdown.push_str(&format!(
            "| {} | {} |\n",
            escape_spec_cell(name),
            escape_spec_cell(summary),
        ));
    }
    writeln!(out, "{}", render::render(&markdown, style))
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
mod tests;
