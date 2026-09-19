//! A negative year renders as a minus sign followed by the zero-padded magnitude
//! (bug-639).
//!
//! `__datetime_padN` left-padded `toString(year)` with zeros to the token width,
//! so the pad went *between* the minus sign and the digits' left edge: year -1
//! rendered as `00-1` for `yyyy`, and `datetime::toIso` built the unparseable
//! stamp `00-1-01-01T00:00:00Z`. The width now applies to the digits and the
//! sign is written in front of them (`-0001`), the way ISO 8601's expanded
//! year representation writes a negative year. Non-negative years, and years
//! with at least as many digits as the width, render exactly as before.

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

/// The expected text for `year` padded to `width` digits.
fn padded(year: i64, width: usize) -> String {
    let digits = format!("{:0width$}", year.unsigned_abs());
    if year < 0 {
        format!("-{digits}")
    } else {
        digits
    }
}

#[test]
fn a_negative_year_renders_its_sign_before_the_padded_digits() {
    let years: [i64; 10] = [-1, -44, -999, -2026, -12345, 0, 7, 44, 2026, 12345];
    let mut body = String::new();
    let mut expected = Vec::new();
    for (index, year) in years.iter().enumerate() {
        body.push_str(&format!(
            "  LET d{index} = datetime::civil(datetime::date({year}, 3, 4), datetime::time(5, 6, 7, 890000000), datetime::utc())\n"
        ));
        for width in [1usize, 3, 4, 5] {
            let token = "y".repeat(width);
            body.push_str(&format!(
                "  io::print(\"{year} {token} \" & datetime::format(d{index}, \"{token}\"))\n"
            ));
            expected.push(format!("{year} {token} {}", padded(*year, width)));
        }
        body.push_str(&format!(
            "  io::print(\"{year} iso0 \" & datetime::toIso(d{index}, 0))\n  io::print(\"{year} iso \" & datetime::toIso(d{index}))\n  io::print(\"{year} full \" & datetime::format(d{index}, \"yyyy-MM-dd HH:mm:ss\"))\n"
        ));
        let y4 = padded(*year, 4);
        expected.push(format!("{year} iso0 {y4}-03-04T05:06:07Z"));
        expected.push(format!("{year} iso {y4}-03-04T05:06:07.890Z"));
        expected.push(format!("{year} full {y4}-03-04 05:06:07"));
    }
    let source = format!("IMPORT io\nIMPORT datetime\n\nSUB main()\n{body}END SUB\n");

    let project = common::temp_project("datetime_negative_year_render", &source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(60),
        "rendering a handful of dates must finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    let actual: Vec<&str> = stdout.lines().collect();
    let wrong: Vec<String> = expected
        .iter()
        .zip(actual.iter().copied().chain(std::iter::repeat("<missing>")))
        .filter(|(e, a)| e.as_str() != *a)
        .map(|(e, a)| format!("expected {e:?}, got {a:?}"))
        .collect();
    assert!(
        wrong.is_empty() && actual.len() == expected.len(),
        "wrong renderings ({} lines, expected {}):\n{}",
        actual.len(),
        expected.len(),
        wrong.join("\n")
    );
}
