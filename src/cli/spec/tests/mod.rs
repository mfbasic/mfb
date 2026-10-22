use super::*;

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// A real package that has at least one topic, plus one of its topics.
fn a_package_with_topic() -> (&'static str, &'static str) {
    let package = spec::packages()
        .iter()
        .find(|package| !package.topics.is_empty())
        .expect("at least one spec package has topics");
    (package.name, package.topics[0].name)
}

#[test]
fn index_ok_with_no_args() {
    assert!(show_spec(&[]).is_ok());
}

#[test]
fn index_forces_color_and_width_flags() {
    assert!(show_spec(&s(&["--no-color", "--width", "60"])).is_ok());
    assert!(show_spec(&s(&["--color", "--width=100"])).is_ok());
}

#[test]
fn package_page_renders() {
    let (package, _) = a_package_with_topic();
    assert!(show_spec(&s(&[package])).is_ok());
}

#[test]
fn package_all_renders_every_subtopic() {
    let (package, _) = a_package_with_topic();
    assert!(show_spec(&s(&[package, "--all"])).is_ok());
}

#[test]
fn subtopic_page_renders() {
    let (package, topic) = a_package_with_topic();
    assert!(show_spec(&s(&[package, topic])).is_ok());
}

#[test]
fn all_without_topic_is_error() {
    let err = show_spec(&s(&["--all"])).unwrap_err();
    assert!(err.contains("--all requires a topic"));
}

#[test]
fn all_with_subtopic_is_error() {
    let (package, topic) = a_package_with_topic();
    let err = show_spec(&s(&[package, topic, "--all"])).unwrap_err();
    assert!(err.contains("--all cannot be combined with a subtopic"));
}

#[test]
fn unknown_package_is_error() {
    let err = show_spec(&s(&["definitely-not-a-package"])).unwrap_err();
    assert!(err.contains("unknown spec topic"));
    assert!(err.contains("Available topics"));
}

#[test]
fn unknown_subtopic_is_error() {
    let (package, _) = a_package_with_topic();
    let err = show_spec(&s(&[package, "not-a-subtopic"])).unwrap_err();
    assert!(err.contains("unknown topic"));
}

#[test]
fn unknown_package_with_subtopic_is_error() {
    let err = show_spec(&s(&["definitely-not-a-package", "topic"])).unwrap_err();
    assert!(err.contains("unknown spec topic"));
    assert!(err.contains("Available topics"));
}

#[test]
fn an_empty_listing_is_accepted() {
    let style = render::Style {
        width: 80,
        color: false,
    };
    let mut out = Vec::new();
    print_spec_listing(&mut out, "Topic", &[], &style).unwrap();
    assert!(out.is_empty());
}

/// A writer whose every write fails with `kind`.
struct FailingWriter(std::io::ErrorKind);

impl Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(self.0))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// bug-787: every rendering mode reports a failed write instead of panicking,
/// and a non-broken-pipe failure keeps its kind so `show_spec` reports it.
#[test]
fn every_rendering_mode_propagates_write_failures() {
    let (package, topic) = a_package_with_topic();
    for args in [
        s(&[]),
        s(&[package]),
        s(&[package, "--all"]),
        s(&[package, topic]),
    ] {
        let (request, style) = parse_spec_request(&args).unwrap();
        let mut out = FailingWriter(std::io::ErrorKind::PermissionDenied);
        let err = write_spec(&mut out, &request, &style).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{args:?}");
    }
}

/// A closed stdout (`mfb spec ... | head`) ends the output quietly; any other
/// write failure is the error `show_spec` reports before exiting 1.
#[test]
fn a_broken_pipe_ends_the_output_quietly_and_any_other_failure_is_reported() {
    let (request, style) = parse_spec_request(&s(&[])).unwrap();
    assert_eq!(
        output_spec(
            &mut FailingWriter(std::io::ErrorKind::BrokenPipe),
            &request,
            &style
        ),
        Ok(())
    );
    assert_eq!(
        output_spec(
            &mut FailingWriter(std::io::ErrorKind::PermissionDenied),
            &request,
            &style
        ),
        Err("failed writing spec output: permission denied".to_string())
    );
    let mut out = Vec::new();
    assert_eq!(output_spec(&mut out, &request, &style), Ok(()));
    assert!(!out.is_empty());
}

/// A package with no subtopics prints its overview alone: no listing and no
/// "Run `mfb spec <package> <subtopic>`" pointer to subtopics it lacks.
#[test]
fn a_package_without_subtopics_prints_only_its_overview() {
    let style = render::Style {
        width: 80,
        color: false,
    };
    let package = spec::SpecPackage {
        name: "lonely",
        summary: "A package with nothing under it.".to_string(),
        overview: "# Lonely\n\nJust this page.\n",
        topics: Vec::new(),
    };
    let mut out = Vec::new();
    print_spec_package(&mut out, &package, &style).unwrap();
    let mut expected = Vec::new();
    writeln!(expected, "{}", render::render(package.overview, &style)).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        String::from_utf8(expected).unwrap()
    );
}

#[test]
fn too_many_positionals_is_error() {
    let err = show_spec(&s(&["a", "b", "c"])).unwrap_err();
    assert!(err.contains("at most two arguments"));
}

#[test]
fn width_flag_requires_a_value() {
    let err = show_spec(&s(&["--width"])).unwrap_err();
    assert!(err.contains("--width requires a number"));
}

#[test]
fn invalid_width_values_are_errors() {
    let err = show_spec(&s(&["--width", "wide"])).unwrap_err();
    assert!(err.contains("invalid --width value"));
    let err = show_spec(&s(&["--width=nope"])).unwrap_err();
    assert!(err.contains("invalid --width value"));
}

#[test]
fn unknown_option_is_error() {
    let err = show_spec(&s(&["--bogus"])).unwrap_err();
    assert!(err.contains("unknown option"));
}

#[test]
fn parse_spec_width_clamps() {
    assert_eq!(parse_spec_width("5").unwrap(), 20);
    assert_eq!(parse_spec_width("50").unwrap(), 50);
    assert_eq!(parse_spec_width("100000").unwrap(), 1000);
}

#[test]
fn detect_terminal_width_honours_columns_override() {
    // `detect_terminal_width` reads `COLUMNS`; run serially-safe by restoring.
    let saved = env::var("COLUMNS").ok();
    env::set_var("COLUMNS", "123");
    assert_eq!(detect_terminal_width(), 123);
    env::set_var("COLUMNS", "999999");
    assert_eq!(detect_terminal_width(), 1000);
    match saved {
        Some(value) => env::set_var("COLUMNS", value),
        None => env::remove_var("COLUMNS"),
    }
}

/// A writer that fails only the write carrying `needle`, accepting the rest.
struct FailingAt(&'static str);

impl Write for FailingAt {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if String::from_utf8_lossy(buf).contains(self.0) {
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A topic page whose closing hint cannot be written reports that failure
/// rather than ending as if it were complete.
#[test]
fn a_failed_closing_hint_on_a_topic_page_is_reported() {
    let (package, _) = a_package_with_topic();
    let (request, style) = parse_spec_request(&s(&[package])).unwrap();
    let err = write_spec(&mut FailingAt("for details."), &request, &style).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
}
