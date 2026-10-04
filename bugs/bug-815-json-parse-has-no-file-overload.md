# bug-815: `json::parse` has no `fs::File` overload

Last updated: 2026-10-04
Effort: small–medium (guess)
Severity: LOW
Class: API gap

Status: Open
Regression Test: proposed `tests/runtime/rt_json_parse_file.rs`, plus a valid fixture
using both new overloads.

## Problem

`json::parse` takes only text:

1. `json::parse(value AS String) AS json::Json`
2. `json::parse(value AS String, reviver AS FUNC(String, json::Json) AS json::Json) AS json::Json`

A program holding an open file must read it fully into a `String` first
(`fs::readAll`) and then parse. `csv::parse` is gaining an `fs::File` overload in
bug-staging-8 (bug-809), and `fs::openData` there makes an in-memory value
available as an `fs::File`. `json` should accept a file the same way.

## Goal

Add, with the owner-specified signatures:

```
json::parse(RES f AS fs::File) AS json::Json
json::parse(RES f AS fs::File, reviver AS FUNC(String, json::Json) AS json::Json) AS json::Json
```

- Parses one complete JSON document from the file's current position to end of
  input. The result, every error and the reviver behave exactly as the `String`
  overloads do for the same text.
- Uses the caller's open `File`: it does not close it. The file's position ends at
  end of input. A closed or moved file raises the usual resource error.
- Declares exactly the errors it raises: the `String` overloads' set, plus the file
  read errors it can reach. The bug-698/807 declared-errors guards must stay green.
- Man page and spec (`stdlib` json topic) updated. Works on `fs::openData` files once
  bug-809 lands.

## Phases

- [ ] RED test: both overloads over a real file and an `openData` file, including a
      malformed document (same error as `String`) and a closed file.
- [ ] Implement, ideally sharing the `String` overloads' parser rather than copying it.
- [ ] Docs, guards, targeted fixtures; goldens in the staging-8 gate.
