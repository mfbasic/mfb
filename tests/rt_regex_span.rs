//! `regex` reports each match's span, its text and its capture groups (bug-532).
//!
//! Before this fixture the package answered only "is there a match" and "where does
//! one start". A pattern's match LENGTH is an output, not something the caller knows
//! in advance, so a start index alone could not be sliced: `findAll("a1b22c333",
//! "\d+")` returned `[1, 3, 6]` and nothing reported that the matches were `"1"`,
//! `"22"` and `"333"`. The only routes to the text were `replace` with a `$0`
//! template into a delimiter the match itself may contain, or re-matching an
//! anchored probe at every candidate length. `regex::findMatch` and
//! `regex::findAllMatches` report it directly.
//!
//! **The positive half is the load-bearing half.** The new members must not be a
//! second matcher: `THE_MATCHER_CORPUS_CROSSCHECK` runs the same 85 pattern/subject
//! pairs `rt_regex_bounds.rs` pins through both the old members and the new ones and
//! requires, per case, that the starts agree one-for-one with `findAll`, that
//! `findMatch(...).start` is `find(...)`, and — the strongest of the three — that
//! `replace`'s output can be REBUILT from the reported spans and groups. That last
//! check reads the subject through `MatchInfo.start`/`endIndex` and `Group.text`
//! while `replace` reads it through the `$N` expander, so a wrong span, a wrong
//! group or a wrong end position cannot agree by accident. `rt_regex_bounds.rs`'s
//! own corpus is untouched and still pins the four original members byte-for-byte.

mod common;

use std::time::Duration;

/// Build a console program and run it under a deadline, returning stdout.
fn run(name: &str, source: &str, timeout: Duration, hang_context: &str) -> String {
    let project = common::temp_project(name, source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(&binary, timeout, hang_context);
    assert!(
        status.success(),
        "{name}: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout
}

fn assert_output(name: &str, got: &str, want: &str) {
    let got = got.trim_end_matches('\n');
    let want = want.trim_end_matches('\n');
    if got != want {
        let first = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| got.lines().count().min(want.lines().count()));
        panic!(
            "{name} differs at line {first}:\n  got:  {:?}\n  want: {:?}\n--- full output ---\n{got}",
            got.lines().nth(first).unwrap_or("<end of output>"),
            want.lines().nth(first).unwrap_or("<end of output>"),
        );
    }
}

const SPANS: &str = r####"IMPORT io
IMPORT regex
IMPORT collections
IMPORT strings

FUNC show(label AS String, m AS regex::MatchInfo) AS String
  MUT s AS String = label & " [" & toString(m.start) & "," & toString(m.endIndex) & ")=" & m.text
  FOR EACH g IN m.groups
    s = s & " (" & toString(g.start) & "," & toString(g.endIndex) & ":" & g.text & ")"
  NEXT
  RETURN s
END FUNC

FUNC probe(label AS String, v AS String, p AS String, at AS Integer) AS Integer
  io::print(show(label, regex::findMatch(v, p, at)))
  RETURN 0
  TRAP(e)
    io::print(label & " raised " & toString(e.code))
    RETURN 1
  END TRAP
END FUNC

SUB main()
  ' The bug's own case: three matches whose lengths differ and which no caller
  ' can know in advance.
  FOR EACH m IN regex::findAllMatches("a1b22c333", "\\d+")
    io::print(show("span", m))
  NEXT

  ' Rewriting around a match is what needs the end index.
  LET text AS String = "a1b22c333"
  LET first AS regex::MatchInfo = regex::findMatch(text, "\\d+")
  io::print("rewrite " & strings::left(text, first.start) & "[" & first.text & "]" & strings::right(text, len(text) - first.endIndex))

  ' Captures by number; a group that took no part in the match is -1/-1/"".
  io::print(show("nonpart", regex::findMatch("ac", "(a)(b)?(c)")))

  ' Captures by name, without counting parentheses.
  LET d AS regex::MatchInfo = regex::findMatch("due 2024-06", "(?<year>\\d{4})-(?<month>\\d{2})")
  io::print("named year=" & collections::get(d.groups, collections::get(d.names, "year")).text & " month=" & collections::get(d.groups, collections::get(d.names, "month")).text & " count=" & toString(len(d.names)) & " hasDay=" & toString(collections::hasKey(d.names, "day")))

  ' The zero-width rule: the same match sequence findAll reports, and it terminates.
  FOR EACH m IN regex::findAllMatches("aba", "a*")
    io::print(show("zw", m))
  NEXT
  io::print("zwStarts=" & toString(len(regex::findAll("aba", "a*"))))
  FOR EACH m IN regex::findAllMatches("abc", "")
    io::print(show("empty", m))
  NEXT

  ' Scalar indices, never byte offsets: the accented scalars ahead of each match
  ' would shift every index if these were UTF-8 offsets.
  FOR EACH m IN regex::findAllMatches("caf\u{e9} 123 na\u{ef}ve 45", "\\d+")
    io::print(show("nonascii", m))
  NEXT

  ' Absence is the -1 sentinel and the empty list, never a failure.
  io::print(show("absent", regex::findMatch("abc", "\\d")) & " groups=" & toString(len(regex::findMatch("abc", "\\d").groups)) & " names=" & toString(len(regex::findMatch("abc", "\\d").names)))
  io::print("absentAll=" & toString(len(regex::findAllMatches("abc", "\\d"))))

  ' start == len(value) is in range; beyond it is not; the pattern is checked first.
  LET r1 AS Integer = probe("atEnd", "abc", "x*", 3)
  LET r2 AS Integer = probe("past", "abc", "x*", 4)
  LET r3 AS Integer = probe("negative", "abc", "x*", -1)
  LET r4 AS Integer = probe("badPattern", "abc", "(", 9)
END SUB
"####;

const SPANS_EXPECTED: &str = r####"span [1,2)=1 (1,2:1)
span [3,5)=22 (3,5:22)
span [6,9)=333 (6,9:333)
rewrite a[1]b22c333
nonpart [0,2)=ac (0,2:ac) (0,1:a) (-1,-1:) (1,2:c)
named year=2024 month=06 count=2 hasDay=FALSE
zw [0,1)=a (0,1:a)
zw [2,3)=a (2,3:a)
zwStarts=2
empty [0,0)= (0,0:)
empty [1,1)= (1,1:)
empty [2,2)= (2,2:)
empty [3,3)= (3,3:)
nonascii [5,8)=123 (5,8:123)
nonascii [15,17)=45 (15,17:45)
absent [-1,-1)= groups=0 names=0
absentAll=0
atEnd [3,3)= (3,3:)
past raised 77050001
negative raised 77050001
badPattern raised 77050003
"####;

/// The headline claim: a caller can obtain each match's span, its text and its
/// capture groups, for a pattern whose match length it cannot know in advance.
///
/// Every line here failed to COMPILE before the fix — `regex` exported neither
/// member and neither type — which is what made extraction impossible rather than
/// merely awkward.
#[test]
fn spans_text_and_captures_are_reported_for_a_variable_length_pattern() {
    let out = run(
        "regex_span_spans",
        SPANS,
        Duration::from_secs(120),
        "the span fixture did not finish",
    );
    assert_output("the span fixture", &out, SPANS_EXPECTED);
}

const OWN_GROUP_TYPE: &str = r####"IMPORT io
IMPORT regex
IMPORT collections

' bug-532 added the EXPORTed record `regex::Group`. A program that already had a
' record of its own by that name, and imports regex, must still compile and mean
' what it always meant -- package types are qualified, so the names do not meet.
TYPE Group
  n AS Integer
END TYPE

FUNC bump(g AS Group) AS Group
  RETURN Group[g.n + 1]
END FUNC

SUB main()
  LET mine AS Group = bump(Group[41])
  io::print("mine=" & toString(mine.n))
  LET m AS regex::MatchInfo = regex::findMatch("ab", "(a)(b)")
  LET theirs AS regex::Group = collections::get(m.groups, 2)
  io::print("theirs=" & theirs.text & " at " & toString(theirs.start))
END SUB
"####;

/// The positive pin on the new type surface: adding `regex::Group` must not take
/// the name `Group` away from a program that already used it.
#[test]
fn a_program_with_its_own_group_record_still_compiles_alongside_regex() {
    let out = run(
        "regex_span_own_group",
        OWN_GROUP_TYPE,
        Duration::from_secs(120),
        "the own-Group fixture did not finish",
    );
    assert_output("the own-Group fixture", &out, "mine=42\ntheirs=b at 1\n");
}

/// The 85 pattern/subject pairs `rt_regex_bounds.rs` pins, run through the OLD
/// members and the NEW ones together. Each case prints `ok` only when every
/// cross-check holds; a mismatch names which one failed rather than the value, so
/// the expected block stays readable.
const CROSSCHECK: &str = r####"IMPORT io
IMPORT regex
IMPORT collections
IMPORT strings

FUNC g(m AS regex::MatchInfo, i AS Integer) AS String
  IF i >= len(m.groups) THEN
    RETURN ""
  END IF
  RETURN collections::get(m.groups, i).text
END FUNC

' Rebuild `replace`'s output from the spans and groups the new members report.
' `replace` splices the matched extents out through the $N expander; this splices
' them out through MatchInfo.start / MatchInfo.endIndex / Group.text. A wrong end
' position shows up as a wrong gap, a wrong group as wrong text.
FUNC recon(subj AS String, pat AS String) AS String
  MUT out AS String = ""
  MUT cursor AS Integer = 0
  FOR EACH m IN regex::findAllMatches(subj, pat)
    out = out & strings::mid(subj, cursor, m.start - cursor)
    out = out & "<" & m.text & "|" & g(m, 1) & "|" & g(m, 2) & "|" & g(m, 3) & ">"
    cursor = m.endIndex
  NEXT
  out = out & strings::mid(subj, cursor, len(subj) - cursor)
  RETURN out
END FUNC

FUNC one(idx AS Integer, pat AS String, subj AS String) AS String
  MUT bad AS String = ""
  LET starts AS List OF Integer = regex::findAll(subj, pat)
  LET ms AS List OF regex::MatchInfo = regex::findAllMatches(subj, pat)
  IF len(starts) <> len(ms) THEN
    bad = bad & " count(" & toString(len(starts)) & "/" & toString(len(ms)) & ")"
  ELSE
    MUT i AS Integer = 0
    WHILE i < len(starts)
      LET m AS regex::MatchInfo = collections::get(ms, i)
      IF collections::get(starts, i) <> m.start THEN
        bad = bad & " start" & toString(i)
      END IF
      IF m.endIndex < m.start THEN
        bad = bad & " span" & toString(i)
      END IF
      IF m.text <> strings::mid(subj, m.start, m.endIndex - m.start) THEN
        bad = bad & " text" & toString(i)
      END IF
      IF len(m.groups) = 0 THEN
        bad = bad & " nogroups" & toString(i)
      ELSE
        LET g0 AS regex::Group = collections::get(m.groups, 0)
        IF g0.start <> m.start OR g0.endIndex <> m.endIndex OR g0.text <> m.text THEN
          bad = bad & " g0" & toString(i)
        END IF
      END IF
      i = i + 1
    END WHILE
  END IF
  IF regex::findMatch(subj, pat).start <> regex::find(subj, pat) THEN
    bad = bad & " first"
  END IF
  IF regex::findMatch(subj, pat, 1).start <> regex::find(subj, pat, 1) THEN
    bad = bad & " first1"
  END IF
  IF recon(subj, pat) <> regex::replace(subj, pat, "<$0|$1|$2|$3>") THEN
    bad = bad & " recon"
  END IF
  IF bad = "" THEN
    RETURN toString(idx) & ": ok"
  END IF
  RETURN toString(idx) & ":" & bad
  TRAP(e)
    RETURN toString(idx) & ": raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(one(0, "abc", "xxabcxxabc"))
  io::print(one(1, "a.c", "abc a\nc a-c"))
  io::print(one(2, "(?s)a.c", "abc a\nc"))
  io::print(one(3, "^abc$", "abc"))
  io::print(one(4, "^abc$", "abcd"))
  io::print(one(5, "(?m)^b", "a\nb\nb"))
  io::print(one(6, "(?m)c$", "c\nc\nd"))
  io::print(one(7, "\\bcat\\b", "cat concat cat"))
  io::print(one(8, "\\Bcat", "cat concat"))
  io::print(one(9, "[a-c]+", "xxabcabcxx"))
  io::print(one(10, "[^a-c]+", "abxyzab"))
  io::print(one(11, "\\d+", "a12b345c"))
  io::print(one(12, "\\w+", "hello, world_1!"))
  io::print(one(13, "\\s+", "a  b\t c"))
  io::print(one(14, "a+", "baaab"))
  io::print(one(15, "a+?", "baaab"))
  io::print(one(16, "a*", "baaa"))
  io::print(one(17, "a*?", "baaa"))
  io::print(one(18, "a{2}", "aaaaa"))
  io::print(one(19, "a{2,}", "aaaaa"))
  io::print(one(20, "a{2,3}", "aaaaaaa"))
  io::print(one(21, "a{2,3}?", "aaaaaaa"))
  io::print(one(22, "colou?r", "color colour colouur"))
  io::print(one(23, "(a|ab)(c|bcd)(d*)", "abcd"))
  io::print(one(24, "(a|ab)(c|bcd)(d*)", "abcd"))
  io::print(one(25, "(ab)+", "ababab abab"))
  io::print(one(26, "(ab)*c", "c abc ababc"))
  io::print(one(27, "(a|b)*", "abba"))
  io::print(one(28, "(?:ab)+", "ababab"))
  io::print(one(29, "(?<year>\\d{4})-(?<month>\\d{2})", "on 2024-06 and 1999-12"))
  io::print(one(30, "(?P<w>\\w+)", "hi there"))
  io::print(one(31, "(?i)hello", "Hello HELLO hello"))
  io::print(one(32, "(?i)[a-c]+", "AbCxaBc"))
  io::print(one(33, "(?i)\u{E9}", "\u{C9} \u{E9} E"))
  io::print(one(34, "\u{E9}+", "caf\u{E9} \u{E9}\u{E9}"))
  io::print(one(35, "\\x{1F600}", "a\u{1F600}b"))
  io::print(one(36, "[\\x{1F600}-\\x{1F64F}]", "a\u{1F600}b\u{1F601}c"))
  io::print(one(37, ".", "a\u{1F600}b"))
  io::print(one(38, "^(\\w+)@(\\w+)\\.com$", "john@example.com"))
  io::print(one(39, "^([a-z0-9-]+\\.)+[a-z]{2,}$", "ab.cd.ef.gh.ij.kl.mn.op.qr.st.uv.wx.yz.com"))
  io::print(one(40, "^([a-z0-9-]+\\.)+[a-z]{2,}$", "ab.cd.ef.gh.ij.kl.mn.op.qr.st.uv.wx.yz."))
  io::print(one(41, "(a+)+b", "aaab"))
  io::print(one(42, "(x+x+)+y", "xxxxxxxxy"))
  io::print(one(43, "(\\d+)(?:px|em)", "10px 2em 3pt"))
  io::print(one(44, "a|ab|abc", "abc"))
  io::print(one(45, "abc|ab|a", "abc"))
  io::print(one(46, "(a*)*", "aaa"))
  io::print(one(47, "(a?)+b", "aab"))
  io::print(one(48, "x*", ""))
  io::print(one(49, "", "abc"))
  io::print(one(50, "\\$\\d", "cost $5 or $7"))
  io::print(one(51, "(\\w)(\\w)", "abcd"))
  io::print(one(52, "(?<a>x)|(?<b>y)", "xy"))
  io::print(one(53, "\\p{L}+", "h\u{E9}llo w\u{F6}rld 123"))
  io::print(one(54, "\\P{L}+", "h\u{E9}llo w\u{F6}rld 123"))
  io::print(one(55, "[[:digit:]]+", "ab12cd3"))
  io::print(one(56, "(?x) a b  c", "abc"))
  io::print(one(57, "a\\tb", "a\tb"))
  io::print(one(58, "[.]", "a.b"))
  io::print(one(59, "\\.", "a.b"))
  io::print(one(60, "(ab|a)(bc|c)?", "abc"))
  io::print(one(61, "^(?:(a)|b)*$", "abab"))
  io::print(one(62, "(a)|(b)", "b"))
  io::print(one(63, "(?U)a+", "aaa"))
  io::print(one(64, "(?U)a+?", "aaa"))
  io::print(one(65, "a{0}", "aaa"))
  io::print(one(66, "(a{2})*", "aaaaa"))
  io::print(one(67, "[a-]+", "a-b--a"))
  io::print(one(68, "[]a]+", "]a]b"))
  io::print(one(69, "(?i)STRASSE", "stra\u{DF}e strasse"))
  io::print(one(70, "\u{DF}", "stra\u{DF}e"))
  io::print(one(71, "(?i)\u{DF}", "STRASSE \u{DF}"))
  io::print(one(72, "(a|b)+?c", "ababc"))
  io::print(one(73, "(?:a|(b))+", "ab"))
  io::print(one(74, "(?:(a)|b)+", "ab"))
  io::print(one(75, "(a)(b)?(c)", "ac"))
  io::print(one(76, "\\d{2,4}?\\d", "12345"))
  io::print(one(77, "(?i)(?-i)a", "Aa"))
  io::print(one(78, "(?i:a)b", "AB Ab aB ab"))
  io::print(one(79, "a(?=b)", "ab"))
  io::print(one(80, "(a)\\1", "aa"))
  io::print(one(81, "[z-a]", "z"))
  io::print(one(82, "(", "a"))
  io::print(one(83, "a{3,2}", "a"))
  io::print(one(84, "*a", "a"))
END SUB
"####;

const CROSSCHECK_EXPECTED: &str = r####"0: ok
1: ok
2: ok
3: ok
4: ok
5: ok
6: ok
7: ok
8: ok
9: ok
10: ok
11: ok
12: ok
13: ok
14: ok
15: ok
16: ok
17: ok
18: ok
19: ok
20: ok
21: ok
22: ok
23: ok
24: ok
25: ok
26: ok
27: ok
28: ok
29: ok
30: ok
31: ok
32: ok
33: ok
34: ok
35: ok
36: ok
37: ok
38: ok
39: ok
40: ok
41: ok
42: ok
43: ok
44: ok
45: ok
46: ok
47: ok
48: raised 77050001
49: ok
50: ok
51: ok
52: ok
53: ok
54: ok
55: ok
56: ok
57: ok
58: ok
59: ok
60: ok
61: ok
62: ok
63: ok
64: ok
65: ok
66: ok
67: ok
68: raised 77050003
69: ok
70: ok
71: ok
72: ok
73: ok
74: ok
75: ok
76: ok
77: ok
78: ok
79: raised 77050003
80: raised 77050003
81: raised 77050003
82: raised 77050003
83: raised 77050003
84: raised 77050003
"####;

/// Every one of the 85 cases must agree, including the eight that raise (an
/// invalid pattern, and `find(subj, pat, 1)` on the empty subject) — the new
/// members raise the same codes at the same cases, so a case going quiet would
/// itself be a difference.
#[test]
fn the_new_members_agree_with_the_index_only_members_across_the_matcher_corpus() {
    let out = run(
        "regex_span_crosscheck",
        CROSSCHECK,
        Duration::from_secs(180),
        "the cross-check corpus did not finish",
    );
    assert_output("the cross-check corpus", &out, CROSSCHECK_EXPECTED);
}
