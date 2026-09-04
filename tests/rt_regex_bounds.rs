//! The regex matcher's cost is bounded by the input, not by the pattern's shape
//! (bug-510, audit-3 DEC-01/02/03).
//!
//! Three defects, one engine. DEC-01: the matcher charged its recursion-depth guard
//! per *frame*, and a group repeat costs ~10 native frames per repetition, so
//! `^(ab)*$` raised "nesting limit exceeded" on a 200-character input and the
//! hostname pattern `^([a-z0-9-]+\.)+[a-z]{2,}$` failed at 54 labels. DEC-02: the
//! backtracking budget reset per *match*, so `findAll`/`replace` cost `matches x
//! budget` — 368 bytes of subject bought 17 s of CPU. DEC-03: `__regex_makeCtx`
//! materialised the subject twice, as a `List OF String` and a `List OF Integer`,
//! at ~750 bytes per character.
//!
//! **The positive half is a pinned corpus.** `MATCHER_CORPUS` runs 85
//! pattern/subject pairs through all four public calls and its expected output was
//! recorded from the compiler *before* the fix. Leftmost-first alternation, greedy
//! versus lazy repeats, capture contents through nested and named groups, case
//! folding, Unicode classes, the empty-match rule, and the eight invalid patterns
//! all have to come out byte-identical — the matcher may change how it recurses,
//! not what it matches.

mod common;

use std::time::Duration;

/// Build a console program and run it under a deadline, returning stdout lines.
fn run(name: &str, source: &str, timeout: Duration, hang_context: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(&binary, timeout, hang_context);
    assert!(
        status.success(),
        "{name}: program {}:\n{stdout}",
        common::exit_description(&status),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout.lines().map(str::to_string).collect()
}

/// `regex::match(subject, pattern)` where the subject is `unit` repeated `count`
/// times (plus `tail`), printed as `match=TRUE|FALSE` or `raised <code>`.
fn match_program(pattern: &str, unit: &str, count: usize, tail: &str) -> String {
    format!(
        r#"IMPORT io
IMPORT regex

FUNC run(pat AS String, s AS String) AS Integer
  LET r AS Boolean = regex::match(s, pat)
  io::print("match=" & toString(r))
  RETURN 0
  TRAP(e)
    io::print("raised " & toString(e.code) & ": " & e.message)
    RETURN 1
  END TRAP
END FUNC

SUB main()
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < {count}
    s = s & "{unit}"
    i = i + 1
  END WHILE
  s = s & "{tail}"
  LET rc AS Integer = run("{pattern}", s)
END SUB
"#
    )
}

#[test]
fn a_group_repeat_over_a_long_input_matches() {
    // DEC-01. Each of these is an ordinary pattern on an ordinary input; before the
    // fix every one raised `ErrInvalidFormat` "nesting limit exceeded", because the
    // depth guard counted native frames and a group repetition costs about ten.
    let cases: [(&str, &str, &str, usize, &str); 4] = [
        ("regex_bounds_ab", "^(ab)*$", "ab", 100, ""),
        ("regex_bounds_csv", "^(\\\\d+,)*\\\\d+$", "12,", 60, "1"),
        (
            "regex_bounds_host",
            "^([a-z0-9-]+\\\\.)+[a-z]{2,}$",
            "abc.",
            60,
            "com",
        ),
        ("regex_bounds_alt", "^(?:a|b)*$", "ab", 1000, ""),
    ];
    for (name, pattern, unit, count, tail) in cases {
        let lines = run(
            name,
            &match_program(pattern, unit, count, tail),
            Duration::from_secs(60),
            "a group repeat over a long input did not finish",
        );
        assert_eq!(
            lines.first().map(String::as_str),
            Some("match=TRUE"),
            "{pattern} over {count}x{unit:?}{tail:?}: {lines:?}",
        );
    }
}

#[test]
fn a_repeat_that_does_not_match_still_fails_cleanly() {
    // The same shapes with a subject they cannot match: the answer is FALSE, not a
    // raise and not a hang. This pins that the bound is on cost, not on success.
    let lines = run(
        "regex_bounds_ab_false",
        &match_program("^(ab)*$", "ab", 100, "x"),
        Duration::from_secs(60),
        "a failing group repeat did not finish",
    );
    assert_eq!(lines.first().map(String::as_str), Some("match=FALSE"));
}

#[test]
fn find_all_cost_is_bounded_for_the_whole_call() {
    // DEC-02. `(a|aa){1,20}b|a` on a run of `a`s: at every start position the first
    // alternative explores up to 2^20 ways of covering the remaining `a`s before
    // failing, then the second alternative matches one character and the cursor
    // advances one position. Each search stays under the per-search budget, so
    // before the fix the whole call cost (positions x ~1M steps) — tens of seconds
    // for sixty characters, and unbounded in the subject length. With one budget
    // for the whole call the program finishes in about a second, either by
    // completing or by raising the budget error; both are bounded, and the deadline
    // is the assertion.
    let source = r#"IMPORT io
IMPORT regex
IMPORT collections

FUNC run(pat AS String, s AS String) AS Integer
  LET hits AS List OF Integer = regex::findAll(s, pat)
  io::print("matches=" & toString(len(hits)))
  RETURN 0
  TRAP(e)
    io::print("raised " & toString(e.code))
    RETURN 1
  END TRAP
END FUNC

SUB main()
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < 60
    s = s & "a"
    i = i + 1
  END WHILE
  LET rc AS Integer = run("(a|aa){1,20}b|a", s)
END SUB
"#;
    let lines = run(
        "regex_bounds_findall_budget",
        source,
        Duration::from_secs(8),
        "findAll spent a fresh backtracking budget on every match",
    );
    let line = lines.first().cloned().unwrap_or_default();
    assert!(
        line == "raised 77050003" || line.starts_with("matches="),
        "unexpected output: {lines:?}",
    );
}

/// The subject the memory case scans: 1.2 MB of `a`, searched for a literal that is
/// not there. The scan itself is one pass; what the test measures is the cost of
/// materialising the subject for the matcher.
const LARGE_SUBJECT: &str = r#"IMPORT io
IMPORT regex

SUB main()
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < 1200000
    s = s & "a"
    i = i + 1
  END WHILE
  LET at AS Integer = regex::find(s, "zzz")
  io::print("find=" & toString(at))
END SUB
"#;

#[cfg(unix)]
#[test]
fn a_large_subject_costs_a_bounded_multiple_of_its_size() {
    // DEC-03, the regex share. The matcher context held every character twice —
    // once as a one-character `String` and once as its scalar — and each list was
    // grown by append, so a 1.2 MB subject cost 907 MB of resident memory before
    // it was searched. The scalar list alone, built the same way, is ~75 bytes per
    // character with the arena's growth garbage; 400 MB is well above that and
    // well below what the doubled context cost.
    let project = common::temp_project("regex_bounds_large_subject", LARGE_SUBJECT);
    let binary = common::build_project(&project);
    let (status, stdout, rss) = common::run_bounded_with_rss(
        &binary,
        Duration::from_secs(120),
        "searching a 1.2 MB subject did not finish",
    );
    let _ = std::fs::remove_dir_all(&project);
    assert!(status.success(), "{}:\n{stdout}", common::exit_description(&status));
    assert_eq!(stdout.trim(), "find=-1");
    let rss = rss.expect("unix reports ru_maxrss");
    assert!(
        rss < 400 * 1024 * 1024,
        "searching a 1.2 MB subject peaked at {} MB of resident memory",
        rss / (1024 * 1024),
    );
}

/// Eighty-five pattern/subject/replacement triples through `match`, `find` (from 0
/// and from 1), `findAll` and `replace`. Generated once; the expected output below
/// was captured from the compiler before bug-510's fix and is today's semantics.
const MATCHER_CORPUS: &str = r####"IMPORT io
IMPORT regex
IMPORT collections
IMPORT strings

FUNC one(idx AS Integer, pat AS String, subj AS String, repl AS String) AS String
  LET m AS Boolean = regex::match(subj, pat)
  LET f AS Integer = regex::find(subj, pat)
  LET f2 AS Integer = regex::find(subj, pat, 1)
  LET all AS List OF Integer = regex::findAll(subj, pat)
  MUT alls AS String = ""
  FOR EACH a IN all
    alls = alls & toString(a) & ","
  NEXT
  LET r AS String = regex::replace(subj, pat, repl)
  RETURN toString(idx) & ": m=" & toString(m) & " f=" & toString(f) & " f1=" & toString(f2) & " all=[" & alls & "] r=" & r
  TRAP(e)
    RETURN toString(idx) & ": raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(one(0, "abc", "xxabcxxabc", "<$0>"))
  io::print(one(1, "a.c", "abc a\nc a-c", "[$0]"))
  io::print(one(2, "(?s)a.c", "abc a\nc", "[$0]"))
  io::print(one(3, "^abc$", "abc", "X"))
  io::print(one(4, "^abc$", "abcd", "X"))
  io::print(one(5, "(?m)^b", "a\nb\nb", "X"))
  io::print(one(6, "(?m)c$", "c\nc\nd", "X"))
  io::print(one(7, "\\bcat\\b", "cat concat cat", "dog"))
  io::print(one(8, "\\Bcat", "cat concat", "dog"))
  io::print(one(9, "[a-c]+", "xxabcabcxx", "<$0>"))
  io::print(one(10, "[^a-c]+", "abxyzab", "<$0>"))
  io::print(one(11, "\\d+", "a12b345c", "#"))
  io::print(one(12, "\\w+", "hello, world_1!", "<$0>"))
  io::print(one(13, "\\s+", "a  b\t c", "_"))
  io::print(one(14, "a+", "baaab", "<$0>"))
  io::print(one(15, "a+?", "baaab", "<$0>"))
  io::print(one(16, "a*", "baaa", "<$0>"))
  io::print(one(17, "a*?", "baaa", "<$0>"))
  io::print(one(18, "a{2}", "aaaaa", "<$0>"))
  io::print(one(19, "a{2,}", "aaaaa", "<$0>"))
  io::print(one(20, "a{2,3}", "aaaaaaa", "<$0>"))
  io::print(one(21, "a{2,3}?", "aaaaaaa", "<$0>"))
  io::print(one(22, "colou?r", "color colour colouur", "<$0>"))
  io::print(one(23, "(a|ab)(c|bcd)(d*)", "abcd", "$1|$2|$3"))
  io::print(one(24, "(a|ab)(c|bcd)(d*)", "abcd", "$3|$2|$1"))
  io::print(one(25, "(ab)+", "ababab abab", "<$1>"))
  io::print(one(26, "(ab)*c", "c abc ababc", "<$0:$1>"))
  io::print(one(27, "(a|b)*", "abba", "<$0:$1>"))
  io::print(one(28, "(?:ab)+", "ababab", "<$0>"))
  io::print(one(29, "(?<year>\\d{4})-(?<month>\\d{2})", "on 2024-06 and 1999-12", "${month}/${year}"))
  io::print(one(30, "(?P<w>\\w+)", "hi there", "[${w}]"))
  io::print(one(31, "(?i)hello", "Hello HELLO hello", "X"))
  io::print(one(32, "(?i)[a-c]+", "AbCxaBc", "<$0>"))
  io::print(one(33, "(?i)\u{E9}", "\u{C9} \u{E9} E", "<$0>"))
  io::print(one(34, "\u{E9}+", "caf\u{E9} \u{E9}\u{E9}", "<$0>"))
  io::print(one(35, "\\x{1F600}", "a\u{1F600}b", "<$0>"))
  io::print(one(36, "[\\x{1F600}-\\x{1F64F}]", "a\u{1F600}b\u{1F601}c", "<$0>"))
  io::print(one(37, ".", "a\u{1F600}b", "<$0>"))
  io::print(one(38, "^(\\w+)@(\\w+)\\.com$", "john@example.com", "$2:$1"))
  io::print(one(39, "^([a-z0-9-]+\\.)+[a-z]{2,}$", "ab.cd.ef.gh.ij.kl.mn.op.qr.st.uv.wx.yz.com", "OK"))
  io::print(one(40, "^([a-z0-9-]+\\.)+[a-z]{2,}$", "ab.cd.ef.gh.ij.kl.mn.op.qr.st.uv.wx.yz.", "OK"))
  io::print(one(41, "(a+)+b", "aaab", "<$1>"))
  io::print(one(42, "(x+x+)+y", "xxxxxxxxy", "<$1>"))
  io::print(one(43, "(\\d+)(?:px|em)", "10px 2em 3pt", "<$1>"))
  io::print(one(44, "a|ab|abc", "abc", "<$0>"))
  io::print(one(45, "abc|ab|a", "abc", "<$0>"))
  io::print(one(46, "(a*)*", "aaa", "<$0:$1>"))
  io::print(one(47, "(a?)+b", "aab", "<$0:$1>"))
  io::print(one(48, "x*", "", "<$0>"))
  io::print(one(49, "", "abc", "-"))
  io::print(one(50, "\\$\\d", "cost $5 or $7", "$$"))
  io::print(one(51, "(\\w)(\\w)", "abcd", "$2$1"))
  io::print(one(52, "(?<a>x)|(?<b>y)", "xy", "[${a}${b}]"))
  io::print(one(53, "\\p{L}+", "h\u{E9}llo w\u{F6}rld 123", "<$0>"))
  io::print(one(54, "\\P{L}+", "h\u{E9}llo w\u{F6}rld 123", "<$0>"))
  io::print(one(55, "[[:digit:]]+", "ab12cd3", "<$0>"))
  io::print(one(56, "(?x) a b  c", "abc", "X"))
  io::print(one(57, "a\\tb", "a\tb", "X"))
  io::print(one(58, "[.]", "a.b", "X"))
  io::print(one(59, "\\.", "a.b", "X"))
  io::print(one(60, "(ab|a)(bc|c)?", "abc", "$1|$2"))
  io::print(one(61, "^(?:(a)|b)*$", "abab", "<$1>"))
  io::print(one(62, "(a)|(b)", "b", "<$1|$2>"))
  io::print(one(63, "(?U)a+", "aaa", "<$0>"))
  io::print(one(64, "(?U)a+?", "aaa", "<$0>"))
  io::print(one(65, "a{0}", "aaa", "<$0>"))
  io::print(one(66, "(a{2})*", "aaaaa", "<$0:$1>"))
  io::print(one(67, "[a-]+", "a-b--a", "<$0>"))
  io::print(one(68, "[]a]+", "]a]b", "<$0>"))
  io::print(one(69, "(?i)STRASSE", "stra\u{DF}e strasse", "<$0>"))
  io::print(one(70, "\u{DF}", "stra\u{DF}e", "<$0>"))
  io::print(one(71, "(?i)\u{DF}", "STRASSE \u{DF}", "<$0>"))
  io::print(one(72, "(a|b)+?c", "ababc", "<$0:$1>"))
  io::print(one(73, "(?:a|(b))+", "ab", "<$1>"))
  io::print(one(74, "(?:(a)|b)+", "ab", "<$1>"))
  io::print(one(75, "(a)(b)?(c)", "ac", "$1|$2|$3"))
  io::print(one(76, "\\d{2,4}?\\d", "12345", "<$0>"))
  io::print(one(77, "(?i)(?-i)a", "Aa", "<$0>"))
  io::print(one(78, "(?i:a)b", "AB Ab aB ab", "<$0>"))
  io::print(one(79, "a(?=b)", "ab", "X"))
  io::print(one(80, "(a)\\1", "aa", "X"))
  io::print(one(81, "[z-a]", "z", "X"))
  io::print(one(82, "(", "a", "X"))
  io::print(one(83, "a{3,2}", "a", "X"))
  io::print(one(84, "*a", "a", "X"))
END SUB
"####;

const MATCHER_CORPUS_EXPECTED: &str = r####"0: m=TRUE f=2 f1=2 all=[2,7,] r=xx<abc>xx<abc>
1: m=TRUE f=0 f1=8 all=[0,8,] r=[abc] a
c [a-c]
2: m=TRUE f=0 f1=4 all=[0,4,] r=[abc] [a
c]
3: m=TRUE f=0 f1=-1 all=[0,] r=X
4: m=FALSE f=-1 f1=-1 all=[] r=abcd
5: m=TRUE f=2 f1=2 all=[2,4,] r=a
X
X
6: m=TRUE f=0 f1=2 all=[0,2,] r=X
X
d
7: m=TRUE f=0 f1=11 all=[0,11,] r=dog concat dog
8: m=TRUE f=7 f1=7 all=[7,] r=cat condog
9: m=TRUE f=2 f1=2 all=[2,] r=xx<abcabc>xx
10: m=TRUE f=2 f1=2 all=[2,] r=ab<xyz>ab
11: m=TRUE f=1 f1=1 all=[1,4,] r=a#b#c
12: m=TRUE f=0 f1=1 all=[0,7,] r=<hello>, <world_1>!
13: m=TRUE f=1 f1=1 all=[1,4,] r=a_b_c
14: m=TRUE f=1 f1=1 all=[1,] r=b<aaa>b
15: m=TRUE f=1 f1=1 all=[1,2,3,] r=b<a><a><a>b
16: m=TRUE f=0 f1=1 all=[0,1,] r=<>b<aaa>
17: m=TRUE f=0 f1=1 all=[0,1,2,3,4,] r=<>b<>a<>a<>a<>
18: m=TRUE f=0 f1=1 all=[0,2,] r=<aa><aa>a
19: m=TRUE f=0 f1=1 all=[0,] r=<aaaaa>
20: m=TRUE f=0 f1=1 all=[0,3,] r=<aaa><aaa>a
21: m=TRUE f=0 f1=1 all=[0,2,4,] r=<aa><aa><aa>a
22: m=TRUE f=0 f1=6 all=[0,6,] r=<color> <colour> colouur
23: m=TRUE f=0 f1=-1 all=[0,] r=a|bcd|
24: m=TRUE f=0 f1=-1 all=[0,] r=|bcd|a
25: m=TRUE f=0 f1=2 all=[0,7,] r=<ab> <ab>
26: m=TRUE f=0 f1=2 all=[0,2,6,] r=<c:> <abc:ab> <ababc:ab>
27: m=TRUE f=0 f1=1 all=[0,] r=<abba:a>
28: m=TRUE f=0 f1=2 all=[0,] r=<ababab>
29: m=TRUE f=3 f1=3 all=[3,15,] r=on 06/2024 and 12/1999
30: m=TRUE f=0 f1=1 all=[0,3,] r=[hi] [there]
31: m=TRUE f=0 f1=6 all=[0,6,12,] r=X X X
32: m=TRUE f=0 f1=1 all=[0,4,] r=<AbC>x<aBc>
33: m=TRUE f=0 f1=2 all=[0,2,] r=<É> <é> E
34: m=TRUE f=3 f1=3 all=[3,5,] r=caf<é> <éé>
35: m=TRUE f=1 f1=1 all=[1,] r=a<😀>b
36: m=TRUE f=1 f1=1 all=[1,3,] r=a<😀>b<😁>c
37: m=TRUE f=0 f1=1 all=[0,1,2,] r=<a><😀><b>
38: m=TRUE f=0 f1=-1 all=[0,] r=example:john
39: m=TRUE f=0 f1=-1 all=[0,] r=OK
40: m=FALSE f=-1 f1=-1 all=[] r=ab.cd.ef.gh.ij.kl.mn.op.qr.st.uv.wx.yz.
41: m=TRUE f=0 f1=1 all=[0,] r=<aaa>
42: m=TRUE f=0 f1=1 all=[0,] r=<xxxxxxxx>
43: m=TRUE f=0 f1=1 all=[0,5,] r=<10> <2> 3pt
44: m=TRUE f=0 f1=-1 all=[0,] r=<a>bc
45: m=TRUE f=0 f1=-1 all=[0,] r=<abc>
46: m=TRUE f=0 f1=1 all=[0,] r=<aaa:>
47: m=TRUE f=0 f1=1 all=[0,] r=<aab:>
48: raised 77050001
49: m=TRUE f=0 f1=1 all=[0,1,2,3,] r=-a-b-c-
50: m=TRUE f=5 f1=5 all=[5,11,] r=cost $ or $
51: m=TRUE f=0 f1=1 all=[0,2,] r=badc
52: m=TRUE f=0 f1=1 all=[0,1,] r=[x][y]
53: m=TRUE f=0 f1=1 all=[0,6,] r=<héllo> <wörld> 123
54: m=TRUE f=5 f1=5 all=[5,11,] r=héllo< >wörld< 123>
55: m=TRUE f=2 f1=2 all=[2,6,] r=ab<12>cd<3>
56: m=TRUE f=0 f1=-1 all=[0,] r=X
57: m=TRUE f=0 f1=-1 all=[0,] r=X
58: m=TRUE f=1 f1=1 all=[1,] r=aXb
59: m=TRUE f=1 f1=1 all=[1,] r=aXb
60: m=TRUE f=0 f1=-1 all=[0,] r=ab|c
61: m=TRUE f=0 f1=-1 all=[0,] r=<a>
62: m=TRUE f=0 f1=-1 all=[0,] r=<|b>
63: m=TRUE f=0 f1=1 all=[0,1,2,] r=<a><a><a>
64: m=TRUE f=0 f1=1 all=[0,] r=<aaa>
65: m=TRUE f=0 f1=1 all=[0,1,2,3,] r=<>a<>a<>a<>
66: m=TRUE f=0 f1=1 all=[0,5,] r=<aaaa:aa>a<:>
67: m=TRUE f=0 f1=1 all=[0,3,] r=<a->b<--a>
68: raised 77050003
69: m=TRUE f=7 f1=7 all=[7,] r=straße <strasse>
70: m=TRUE f=4 f1=4 all=[4,] r=stra<ß>e
71: m=TRUE f=8 f1=8 all=[8,] r=STRASSE <ß>
72: m=TRUE f=0 f1=1 all=[0,] r=<ababc:b>
73: m=TRUE f=0 f1=1 all=[0,] r=<b>
74: m=TRUE f=0 f1=1 all=[0,] r=<a>
75: m=TRUE f=0 f1=-1 all=[0,] r=a||c
76: m=TRUE f=0 f1=1 all=[0,] r=<123>45
77: m=TRUE f=1 f1=1 all=[1,] r=A<a>
78: m=TRUE f=3 f1=3 all=[3,9,] r=AB <Ab> aB <ab>
79: raised 77050003
80: raised 77050003
81: raised 77050003
82: raised 77050003
83: raised 77050003
84: raised 77050003
"####;

#[test]
fn matching_semantics_are_unchanged() {
    let lines = run(
        "regex_bounds_corpus",
        MATCHER_CORPUS,
        Duration::from_secs(120),
        "the matcher corpus did not finish",
    );
    let got = lines.join("\n");
    let want = MATCHER_CORPUS_EXPECTED.trim_end_matches('\n');
    if got != want {
        let first = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "the matcher corpus changed at line {first}:\n  got:  {:?}\n  want: {:?}",
            got.lines().nth(first).unwrap_or(""),
            want.lines().nth(first).unwrap_or(""),
        );
    }
}
