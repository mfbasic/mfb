### 1. `datetime::ZoneKind::Utc` spelling on three pages is a parse error
UNIT:      man-pkg:datetime
PAGE:      fixedOffset, offsetAt, utc (vs. inZone, local, withZone, types)
CATEGORY:  consistency
CLAIM:     "The returned datetime::Zone carries a zone kind of datetime::ZoneKind::Utc" (utc); "datetime::ZoneKind::FixedOffset" (fixedOffset); "(datetime::ZoneKind::Utc) and a fixed-offset zone (datetime::ZoneKind::FixedOffset ... datetime::ZoneKind::Local" (offsetAt). The inZone, local, withZone and types pages write the same members as `datetime::ZoneKind.Utc`.
VERDICT:   inconsistent
EVIDENCE:  `grep -n "ZoneKind::" src/codegen/builtins/datetime/*.rs` hits func_fixed_offset.rs:41, func_offset_at.rs:18,20, func_utc.rs:10,12; `grep -n "ZoneKind\." ...` hits func_in_zone.rs, func_local.rs, func_with_zone.rs, mod.rs. Probe kind_colons.mfb (`IF z.kind = datetime::ZoneKind::Utc`) fails to build: `error[1-102-0005 MFB_PARSE_UNEXPECTED_TOKEN]: Package-qualified names must have exactly two parts.` `mfb spec language types` §4.5: "Enum members are addressed as EnumType.Member".
SUGGESTED: Replace every `datetime::ZoneKind::X` on the fixedOffset, offsetAt and utc pages with `datetime::ZoneKind.X`, the only spelling that compiles.

### 2. No page says how to test a zone's kind, and the obvious comparison is a type error
UNIT:      man-pkg:datetime
PAGE:      types (datetime::Zone), utc, fixedOffset, local, offsetAt
CATEGORY:  coverage
CLAIM:     "kind | Integer | Which kind of zone this is: datetime::ZoneKind.Utc, datetime::ZoneKind.FixedOffset, or datetime::ZoneKind.Local ... The field is a plain Integer." Five pages tell the reader the kind is one of those enum members; none says how to compare the field to one.
VERDICT:   missing
EVIDENCE:  Probe kind_dot.mfb (`IF z.kind = datetime::ZoneKind.Utc`): `error[2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH]: Operator = requires compatible comparable operands, got Integer and datetime.ZoneKind.` Probe toint.mfb `z.kind = toInt(datetime::ZoneKind.Local)` on `datetime::local()` prints TRUE. The package's own example code tests the raw number (`IF z.kind = 2` in func_offset_at.rs:71, `dt.zone.kind <> 2` in func_add_days.rs:100), which no page documents.
SUGGESTED: On the Zone record's `kind` description add: "Compare it with toInt(datetime::ZoneKind.Local) (0 Utc, 1 FixedOffset, 2 Local); the field is an Integer, so it cannot be compared to the enum member directly."

### 3. `datetime::Month` is advertised but nothing produces, accepts, or matches it
UNIT:      man-pkg:datetime
PAGE:      package-wide (overview, types)
CATEGORY:  overview-mismatch
CLAIM:     Overview: "All public types are flat, copyable value records and enums — ... and the enums datetime::ZoneKind, datetime::Weekday, and datetime::Month." Weekday has datetime::weekday; Month has no function at all, and the month a developer actually holds (dt.date.month, the month argument of date/daysInMonth) is an Integer.
VERDICT:   misleading
EVIDENCE:  `grep -n "Month\." src/codegen/builtins/datetime/func_*.rs` returns nothing; no declaration in the function table takes or returns datetime::Month. Probe month_enum.mfb (`dt.date.month = datetime::Month.March`): `TYPE_BINARY_OPERATOR_MISMATCH ... got Integer and datetime.Month`. Probe toint.mfb: `dt.date.month = toInt(datetime::Month.March) + 1` prints TRUE, so the enum's ordinal is one less than the month number every page uses, and `toString(datetime::Month.March)` prints `March`.
SUGGESTED: On the Month enum (types page) and in the overview sentence, state its only use and the trap: "No function returns or accepts datetime::Month; date fields and arguments are month numbers 1..12. Convert with toInt(datetime::Month.March) + 1 (ordinals start at 0), or use toString(member) for the English name."

### 4. The overview's list of host-reading operations is incomplete, so "Everything else is a pure function" is false
UNIT:      man-pkg:datetime
PAGE:      overview
CATEGORY:  overview-mismatch
CLAIM:     "Every operation that resolves a datetime::local() zone reads the host's zone rules: localOffset, civil, inZone/toLocal, addDays, and addMonths. ... Everything else is a pure function of its arguments."
VERDICT:   wrong
EVIDENCE:  Probe tz_probe.mfb run twice: under `TZ=UTC` it prints `0 / 2026-03-07 12:00 +00:00 / 2026-03-07 00:00 +00:00 / 2026-07-01T12:00:00.000Z`; under `TZ=America/New_York` it prints `-18000 / 2026-03-07 07:00 -05:00 / 2026-03-07 00:00 -05:00 / 2026-07-01T12:00:00.000-04:00`. Those four lines are offsetAt(local(), at), withZone(dt, local()), startOfDay(local dt) and parse(..., local()) — none is in the overview's list, and each function's own page says it reads the host rules (offsetAt: "reads the host zone table"; withZone: "for a local zone the result uses the host's time-zone rules"; startOfDay: "resolved from the platform's zone table"; parse: three-argument overload "resolves value's civil fields against the given zone").
SUGGESTED: "...reads the host's zone rules: localOffset, offsetAt, civil, inZone/toLocal/withZone, startOfDay, addDays, addMonths, and parse when given datetime::local()."

### 5. No page tells the reader how to shift a DateTime by a Duration
UNIT:      man-pkg:datetime
PAGE:      add, subtract, addDays, withZone
CATEGORY:  coverage
CLAIM:     add/subtract: "For civil, zone-aware day and month arithmetic ... use datetime::addDays and datetime::addMonths on a datetime::DateTime instead." That is the only pointer from Instant arithmetic to DateTime, and it covers only whole days and months. A developer holding a DateTime who wants "two and a half hours later" is never told the route.
VERDICT:   missing
EVIDENCE:  `grep -ln "inZone(datetime::add\|shift a .*DateTime by a .*Duration" src/codegen/builtins/datetime/func_*.rs` matches only func_with_zone.rs, and that page describes re-projection, not shifting. Probe enum_int.mfb `datetime::inZone(datetime::add(datetime::resolve(dt), datetime::duration(2, 30, 0, 0)), dt.zone)` on 2026-03-07T12:00Z prints `2026-03-07T14:30:00.000Z`, so the recipe works but nothing documents it.
SUGGESTED: On the add page (and mirrored on subtract): "To shift a datetime::DateTime by a span, resolve it, shift the instant, and project back into its zone: datetime::inZone(datetime::add(datetime::resolve(dt), by), dt.zone). addDays and addMonths exist because that route counts physical seconds, not calendar days."

### 6. The overview narrative never mentions the calendar-query, comparison, or epoch-conversion groups
UNIT:      man-pkg:datetime
PAGE:      overview
CATEGORY:  discoverability
CLAIM:     The Description's "Projection is the primary ... Arithmetic operates on ... Formatting and parsing share ..." paragraph is the reader's map of the package. It names projection, arithmetic, and formatting only.
VERDICT:   misleading
EVIDENCE:  `grep -c` of the overview text in mod.rs MODULE_DESC for each name: weekday, dayOfYear, daysInMonth, isLeapYear, startOfDay, compare, equals, isBefore, isAfter, toMillis, fromMillis, toNanos, withZone, offsetAt all appear 0 times in the Description (rendered output above shows them only in the alphabetical function table). That is 14 of 44 functions, three whole groups, reachable only by scanning the table.
SUGGESTED: Add one sentence per missing group to the map paragraph: "Calendar queries read a DateTime's civil fields: weekday, dayOfYear, startOfDay, plus daysInMonth and isLeapYear on bare numbers. compare, equals, isBefore and isAfter order instants (resolve a DateTime first). toMillis/fromMillis and toNanos convert an Instant to and from epoch counts. withZone re-projects a DateTime into another zone and offsetAt reports a zone's offset at an instant."
