//! `net::parseQuery` — descriptor entry + the `__net_parseQuery` MFBASIC source body
//! (`Body::mfb`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Parse a URL query string into a map of decoded keys and values."#;

const DESC: &str = r#"`net::parseQuery` parses an `a=1&b=2` query string into a
`Map OF String TO String`. The leading `?` must already have been stripped by the
caller — `net::toUrl` does exactly that, storing the raw query without it, so
`net::parseQuery(net::toUrl(href).query)` is the intended pairing. An empty input
returns an empty map.

The input is split on `&`, and each pair is split at its first `=`. The part
before the `=` is the key and the part after is the value; a bare key with no `=`
at all maps to the empty string, which is how a valueless flag such as `?debug`
appears. An empty pair — produced by `&&`, or by a leading or trailing `&` — is
skipped rather than yielding an empty key. Repeated keys collapse last-wins: the
final occurrence in the string is the one in the map.

Keys and values are both query-decoded: `%XX` escapes become the bytes they name
and a literal `+` becomes a space, which is `application/x-www-form-urlencoded`
semantics. Note that the `+` rule applies to keys as well as values, and that it
is exactly the rule `net::percentDecode` does *not* apply, since a `+` in a path
segment is a literal `+`.

Decoding here is **tolerant**, which is the deliberate difference from
`net::percentDecode`. A component whose escapes are malformed — a truncated `%`,
a non-hexadecimal pair, or bytes that do not form valid UTF-8 — is kept as its
raw undecoded text instead of failing, so `"k=%2"` yields the value `"%2"`. One
bad component therefore never sinks an otherwise valid query, which is what lets
the built-in `http` server route framing errors to a 400 response without letting
soft query-decode failures do the same."#;

const EX: &str = r#"Parse a query and read its values:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  LET q = net::parseQuery("name=a+b&n=42&raw=%2Fx")
  io::print(collections::getOr(q, "name", "?"))
  io::print(collections::getOr(q, "n", "?"))
  io::print(collections::getOr(q, "raw", "?"))
  RETURN 0
END FUNC
```

Parse the query carried by a URL, including a bare key:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  LET u AS net::Url = net::toUrl("https://example.com/search?q=a+b&debug")
  LET q = net::parseQuery(u.query)
  io::print(collections::getOr(q, "q", "?"))
  io::print(toString(len(collections::keys(q))))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"' Parse an `a=1&b=2` query string into a map. Keys and values are
' query-decoded (`+` -> space, `%XX`). A bare `key` (no `=`) maps to `""`; an
' empty pair (from `&&` or a leading/trailing `&`) is skipped. Repeated keys
' collapse last-wins. The leading `?` must already be stripped by the caller.
FUNC __net_parseQuery(s AS String) AS Map OF String TO String
  MUT result AS Map OF String TO String = Map OF String TO String {}
  IF s = "" THEN
    RETURN result
  END IF
  LET pairs AS List OF String = strings::split(s, "&")
  FOR EACH pair IN pairs
    IF pair <> "" THEN
      LET eq AS Integer = __net_indexOf(pair, "=", 0)
      MUT rawKey AS String = pair
      MUT rawValue AS String = ""
      IF eq >= 0 THEN
        rawKey = __net_slice(pair, 0, eq)
        rawValue = __net_slice(pair, eq + 1, len(pair))
      END IF
      LET key AS String = __net_decodeQueryComponent(rawKey)
      LET value AS String = __net_decodeQueryComponent(rawValue)
      result = collections::set(result, key, value)
    END IF
  NEXT
  RETURN result
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "parseQuery",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req("s", "The query string, without its leading `?`. Also accepted under the alternate named-argument spellings `query` and `value`, so `net::parseQuery(s := q)`, `net::parseQuery(query := q)`, and `net::parseQuery(value := q)` all bind position 0.", &["query", "value"], ParameterType::String)],
            return_type: ParameterType::map_of(ParameterType::String, ParameterType::String),
            errors: vec![],
            body: Body::mfb(BODY, "__net_parseQuery"),
        }],
    });
}
