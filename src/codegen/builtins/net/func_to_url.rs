//! `net::toUrl` — descriptor entry + the `__net_toUrl` MFBASIC source body
//! (`Body::mfb`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Parse an absolute http or https URL into its components."#;

const DESC: &str = r#"`net::toUrl` parses an absolute URL of the shape
`scheme://[user[:pass]@]host[:port]path[?query][#fragment]` into a `net::Url` value
record. Unlike `Socket` and its siblings, `net::Url` is an ordinary copyable record,
not a resource handle.

Parsing splits at the first `://`. The scheme before it is lowercased and must be
`http` or `https`; anything else raises `ErrUnsupported`, and a missing `://`
raises `ErrInvalidFormat`. The authority runs to the first `/`, `\`, `?`, or `#`,
or to the end of the string.

A backslash counts as a slash, as it does in a browser. It ends the authority, so
`http://good.com\@evil.com/` has host `good.com` and path `/@evil.com/` — the text
before the backslash is never read as userinfo — and every `\` in the path is
stored as `/`. A backslash in the query or the fragment is kept.

Userinfo is optional and is split off at the **last** `@` in the authority, not
the first, as RFC 3986 and the WHATWG URL standard require. That matters for an
authority carrying more than one `@`: `http://a@b@c/` yields username `a@b` and
host `c`, not host `b@c`. Within the userinfo the split is at the *first* colon:
before it is `username`, after it is `password`, both stored exactly as written
with no decoding. Userinfo with no colon is a username only.

The host must be a name, an IPv4 address, or a bracketed IPv6 address, and it is
checked here, not later when something connects. Anything else raises
`ErrInvalidFormat`, as does an empty host.

A name may not contain a control character, a space, or any of
`#%/:<>?@[\]^|`. That is checked after `%XX` escapes are decoded, so `%00` and a
malformed escape such as `%zz` fail too. A tab, line feed or carriage return in
the host is one of those control characters: `toUrl` removes nothing from the text
you give it. A valid name is stored exactly as written, escapes and letter case
included.

A host whose last dot-separated part is a number is an IPv4 address and must be a
valid one. Each part may be decimal, octal with a leading `0`, or hexadecimal with
a leading `0x`, and there may be one to four parts, the last filling the bytes the
others leave. The stored host is always the plain dotted-decimal address:
`http://0x7f.1/`, `http://2130706433/`, `http://127.1/` and `http://0177.0.0.1/`
all store `127.0.0.1`, and `127.0.0.010` stores `127.0.0.8`. A check you write
against `host` therefore sees the address a connection would reach. A part that
is out of range, a digit that does not belong to its base (`1.2.3.09`), or a name
ending in a number (`foo.09`) raises `ErrInvalidFormat`.

A bracketed host must be an IPv6 address: groups of one to four hexadecimal
digits separated by colons, at most one `::`, optionally ending in a dotted-decimal
IPv4 address. The brackets are stripped, so `[::1]` stores host `::1`, and the
address is otherwise stored as written. After a bracketed literal only a `:port`
may follow — anything else raises `ErrInvalidFormat`, as does an unterminated
bracket.

A host containing a character outside ASCII is stored as written; `toUrl` does
not convert an internationalized name to its `xn--` form, and it does not check
that an `xn--` label is a valid one. The number rule above still applies: such a
host whose last part is a number is refused, because its other parts are not
numbers.

The port is optional and defaults to the scheme default — 443 for `https` and 80
for everything else, which given the scheme check means 80 for `http`. A bare
trailing `:` with nothing after it counts as absent, so
`http://example.com:` takes the default 80 rather than failing.

A **non-empty** explicit port must be ASCII digits only — so no leading `+` or
`-` — and must not exceed 65535; each of those raises `ErrInvalidFormat`. `ErrOutOfMemory` means the program ran out of memory.

What remains is split at the first `#` into a fragment and at the first `?` into a
query, each stored without its leading punctuation. An absent path becomes `"/"`.
Apart from the IPv4 host and the path's backslashes, `toUrl` stores each part as
written: nothing is percent-decoded and letter case is kept — use
`net::percentDecode` for a path component and `net::parseQuery` for the query
string. A universal `toString` on a `net::Url` renders it back to an href, omitting a
port equal to the scheme default and re-bracketing a host containing a colon."#;

const EX: &str = r#"Parse a full URL and read its parts:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  LET u AS net::Url = net::toUrl("https://api.example.com:8443/v1/items?limit=10#frag")
  io::print(u.host)
  io::print(toString(u.port))
  io::print(u.path)
  io::print(u.query)
  RETURN 0
END FUNC
```

Scheme defaults and round-tripping through `toString`:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  LET u AS net::Url = net::toUrl("http://example.com")
  io::print(toString(u.port))
  io::print(u.path)
  io::print(toString(u))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r##"FUNC __net_toUrl(href AS String) AS Url
  LET schemeEnd AS Integer = __net_indexOf(href, "://", 0)
  IF schemeEnd < 0 THEN
    FAIL error(77050003, "invalid URL: missing scheme separator")
  END IF
  LET scheme AS String = strings::lower(__net_slice(href, 0, schemeEnd))
  IF scheme <> "http" AND scheme <> "https" THEN
    FAIL error(77050007, "unsupported URL scheme")
  END IF

  LET rest AS String = __net_slice(href, schemeEnd + 3, len(href))
  LET authorityEnd AS Integer = __net_authorityEnd(rest)
  LET authority AS String = __net_slice(rest, 0, authorityEnd)
  LET pathPart AS String = __net_slice(rest, authorityEnd, len(rest))

  ' Userinfo (user[:pass]@) is optional.
  MUT username AS String = ""
  MUT password AS String = ""
  MUT hostport AS String = authority
  ' bug-306 S3: the LAST `@` is the userinfo/host boundary (RFC 3986 §3.2,
  ' WHATWG URL). Splitting on the first left an `@` inside the host for an
  ' authority carrying more than one -- `http://a@b@c/` produced host `b@c`, which
  ' then passed host parsing unvalidated.
  LET atIndex AS Integer = __net_lastIndexOf(authority, "@")
  IF atIndex >= 0 THEN
    LET userinfo AS String = __net_slice(authority, 0, atIndex)
    hostport = __net_slice(authority, atIndex + 1, len(authority))
    LET colon AS Integer = __net_indexOf(userinfo, ":", 0)
    IF colon >= 0 THEN
      username = __net_slice(userinfo, 0, colon)
      password = __net_slice(userinfo, colon + 1, len(userinfo))
    ELSE
      username = userinfo
    END IF
  END IF

  ' Host (DNS name, IPv4, or bracketed IPv6) and optional port.
  MUT host AS String = ""
  MUT portText AS String = ""
  MUT bracketed AS Boolean = FALSE
  IF strings::startsWith(hostport, "[") THEN
    LET closeBracket AS Integer = __net_indexOf(hostport, "]", 0)
    IF closeBracket < 0 THEN
      FAIL error(77050003, "invalid URL: unterminated IPv6 literal")
    END IF
    host = __net_slice(hostport, 1, closeBracket)
    bracketed = TRUE
    LET afterBracket AS String = __net_slice(hostport, closeBracket + 1, len(hostport))
    IF strings::startsWith(afterBracket, ":") THEN
      portText = __net_slice(afterBracket, 1, len(afterBracket))
    ELSEIF afterBracket <> "" THEN
      FAIL error(77050003, "invalid URL: trailing characters after IPv6 literal")
    END IF
  ELSE
    LET colon AS Integer = __net_indexOf(hostport, ":", 0)
    IF colon >= 0 THEN
      host = __net_slice(hostport, 0, colon)
      portText = __net_slice(hostport, colon + 1, len(hostport))
    ELSE
      host = hostport
    END IF
  END IF
  IF host = "" THEN
    FAIL error(77050003, "invalid URL: empty host")
  END IF
  ' bug-979: the host is validated here, and a numeric IPv4 host is replaced by
  ' its canonical dotted-decimal form, so what a caller compares is what a
  ' connection reaches.
  host = __net_host(host, bracketed)

  MUT port AS Integer = __net_defaultPort(scheme)
  IF portText <> "" THEN
    port = __net_parsePort(portText)
  END IF

  ' Path, query, and fragment.
  MUT pathQueryFragment AS String = pathPart
  MUT fragment AS String = ""
  LET hashIndex AS Integer = __net_indexOf(pathQueryFragment, "#", 0)
  IF hashIndex >= 0 THEN
    fragment = __net_slice(pathQueryFragment, hashIndex + 1, len(pathQueryFragment))
    pathQueryFragment = __net_slice(pathQueryFragment, 0, hashIndex)
  END IF
  MUT query AS String = ""
  MUT path AS String = pathQueryFragment
  LET queryIndex AS Integer = __net_indexOf(pathQueryFragment, "?", 0)
  IF queryIndex >= 0 THEN
    query = __net_slice(pathQueryFragment, queryIndex + 1, len(pathQueryFragment))
    path = __net_slice(pathQueryFragment, 0, queryIndex)
  END IF
  IF path = "" THEN
    path = "/"
  END IF
  ' bug-979: an http(s) path reads `\` as `/` (WHATWG). The query and the fragment
  ' keep theirs.
  IF strings::contains(path, "\\") THEN
    path = strings::replace(path, "\\", "/")
  END IF

  RETURN Url[scheme, username, password, host, port, path, query, fragment]
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toUrl",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("href", "The absolute URL to parse. Also accepted under the alternate named-argument spellings `value` and `url`, so `net::toUrl(href := s)`, `net::toUrl(value := s)`, and `net::toUrl(url := s)` all bind position 0.", &["value", "url"], ParameterType::String)],
            return_type: ParameterType::named(super::URL_TYPE),
            errors: vec!["ErrInvalidFormat", "ErrOutOfMemory", "ErrUnsupported"],
            body: Body::mfb(BODY, "__net_toUrl"),
        }],
    });
}
