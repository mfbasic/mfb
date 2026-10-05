# currency

`Money` amounts tagged with a currency, and the exchange rates that convert between
them. A `currency::Amount` is a `Money` value and a currency code. The package keeps a
table of rates and uses it to add, subtract, compare and convert amounts across
currencies.

```
IMPORT currency
IMPORT io

FUNC main() AS Nothing
  currency::setRate("USD", 1F)      ' the base currency
  currency::setRate("EUR", 1.25F)   ' one euro is worth 1.25 dollars

  LET price AS currency::Amount = currency::amount(5m, "USD")
  LET fee AS currency::Amount = currency::amount(4m, "eur")
  io::print(currency::format(currency::add(price, fee)))   ' 10.00 USD
END FUNC
```

Full API and prose: `mfb pkg doc packages/currency/currency.mfp`, or
`mfb doc packages/currency` for the internals too.

## API

| Member | What it does | Fails with |
|---|---|---|
| `currency::Amount` | A record: `value AS Money`, `code AS String` | — |
| `currency::amount(value, code) AS Amount` | Build an amount; the code is checked and upper-cased | `ErrorInvalidCode` |
| `currency::isValidCode(code) AS Boolean` | Whether `code` is accepted, ignoring case | — |
| `currency::normalizeCode(code) AS String` | The upper-case spelling of a code | `ErrorInvalidCode` |
| `currency::isoCodes() AS List OF String` | Every ISO 4217 code the package accepts | — |
| `currency::isoListDate() AS String` | The publication date of the ISO list it carries | — |
| `currency::minorUnits(code) AS Integer` | Decimal places the currency is booked to | `ErrorInvalidCode` |
| `currency::setRate(code, rate)` | Add or replace a rate | `ErrorInvalidCode`, `ErrorInvalidRate` |
| `currency::removeRate(code)` | Remove a rate; a code with no rate is ignored | `ErrorInvalidCode` |
| `currency::clearRates()` | Empty the table | — |
| `currency::hasRate(code) AS Boolean` | Whether the table has a rate for `code` | `ErrorInvalidCode` |
| `currency::getRate(code) AS Fixed` | The rate for `code` | `ErrorInvalidCode`, `ErrorUnknownCurrency` |
| `currency::hasBaseCurrency() AS Boolean` | Whether some code has rate `1` | — |
| `currency::exportRates() AS Map OF String TO Fixed` | A copy of the table | — |
| `currency::importRates(table)` | Check every entry, then replace the whole table | `ErrorInvalidCode`, `ErrorInvalidRate` |
| `currency::convert(a, code) AS Amount` | `a` expressed in another currency | two-amount rules |
| `currency::add(a, b)` / `subtract(a, b) AS Amount` | `a ± b`, in `a`'s currency | two-amount rules |
| `currency::compare(a, b) AS Integer` | -1, 0 or 1 by worth | two-amount rules |
| `currency::multiply(a, k)` / `divide(a, k) AS Amount` | Scale by an `Integer`, `Fixed` or `Float` | `ErrorInvalidCode` |
| `currency::negate(a) AS Amount` | `-a` | `ErrorInvalidCode` |
| `currency::round(a)` / `round(a, decimals) AS Amount` | Round to the currency's minor units, or to `decimals` | `ErrorInvalidCode` |
| `currency::format(a) AS String` | Text such as `12.50 USD` | `ErrorInvalidCode` |

Every function that takes an `Amount` checks its code, so an `Amount` built directly as
`currency::Amount[...]` is held to the same rules as one from `currency::amount`.

## Currency codes

A code is one of two kinds:

- **An ISO 4217 code**, such as `USD`, `EUR` or `JPY`, that is in the ISO 4217 list
  committed with the package (`third_party/iso4217/`). The ISO `X` codes count, so gold
  (`XAU`), the SDR (`XDR`) and the testing code (`XTS`) are valid. A three-letter code
  that is not in the list, such as `ABC`, or a withdrawn one such as the Croatian kuna
  `HRK`, is rejected.
- **A custom code**: `_` followed by 1 to 15 ASCII letters or digits, such as `_GOLD`,
  `_XWY` or `_POINTS`. Use one for anything ISO does not list: an in-game currency,
  loyalty points, a commodity, or a non-currency rate. ISO codes never start with `_`,
  so a custom code cannot collide with a currency a later ISO list adds.

Codes are case-insensitive. Every code is upper-cased before it is checked, and the
upper-case spelling is the only one the package stores or returns: `"usd"`, `"Usd"`
and `"USD"` are the same code, and `"_gold"` is `_GOLD`. Only ASCII letters are
case-folded, so a code holding a non-ASCII letter is rejected even when its upper-case
form would be ASCII.

The ISO list is part of the package, not looked up at run time: every program built
with the same package version accepts the same codes. ISO adds and withdraws codes a
few times a year, and a newer list arrives as a newer version of this package
(`tools/iso4217/README.md` describes the update).

## The rates table

A rate says how many units of the **base currency** one unit of a code is worth. The
base currency is any code with rate exactly `1`; there is no separate setting for it.
With `USD` as the base:

```
currency::setRate("USD", 1F)        ' base
currency::setRate("EUR", 1.25F)     ' 1 EUR = 1.25 USD
currency::setRate("JPY", 0.0067F)   ' 1 JPY = 0.0067 USD
currency::setRate("_GOLD", 64F)     ' 1 _GOLD = 64 USD
```

Converting an amount from currency F to currency T computes
`value * rate(F) / rate(T)`. The base currency's name is never needed, so changing
which code is the base only means rescaling every rate. Multiplying first keeps the
precision of a small rate; the cost is that the amount's worth in the base currency
must itself fit in `Money`, or the conversion fails with `errorCode::ErrOverflow`.

`setRate` replaces a code's existing rate. A rate must be greater than zero
(`ErrorInvalidRate`). `removeRate` on a code with no rate does nothing.

Rates are `Fixed`, which is binary fixed-point with a resolution of 1/2^32 (about
0.00000000023), not decimal. `1.25F` and `64F` are stored exactly, but most decimal
rates such as `1.0842F` are stored as the nearest binary value. For small rates that
costs relative precision: a rate near `0.00002` keeps only about five significant
digits.

### Rules for two amounts

`add`, `subtract`, `compare` and `convert` work on two currencies (for `convert`, the
amount's and the target's). They check, in this order:

1. **Both codes must have a rate**, even when they are the same code. Otherwise the
   call fails with `ErrorUnknownCurrency`. The table is the list of currencies a
   program deals in, so a misspelled or unexpected code fails instead of quietly
   adding to itself. Adding two `USD` amounts needs `USD` in the table.
2. **If the codes differ, some code must have rate `1`.** Otherwise the call fails
   with `ErrorNoBaseCurrency`. When both checks fail, `ErrorUnknownCurrency` is the
   one reported, because it names the code to fix.

The result is in the first amount's currency: `add(5 USD, 4 EUR)` is in `USD`, and
`add(4 EUR, 5 USD)` is in `EUR`.

### Rules for one amount

`amount`, `multiply`, `divide`, `negate`, `round` and `format` never read the table, so
they work before any rate is set. They still check the code's format.

### Threads

The table belongs to the thread that set it. A worker thread starts with an **empty**
table: every started thread gets its own fresh copy of its project's module state
(`mfb spec language threads`). To give a worker the same rates, send it
`currency::exportRates()` and call `currency::importRates` there:

```
ISOLATED FUNC worker(self AS ThreadWorker OF String TO String, saved AS Map OF String TO Fixed) AS String
  currency::importRates(saved)
  RETURN currency::format(currency::convert(currency::amount(10m, "EUR"), "USD"))
END FUNC

FUNC main() AS Nothing
  currency::setRate("USD", 1F)
  currency::setRate("EUR", 1.25F)
  RES t AS Thread OF String TO String = thread::start(worker, currency::exportRates())
  io::print(thread::waitFor(t))   ' 12.50 USD
END FUNC
```

`importRates` checks every entry with the same rules as `setRate` and upper-cases the
keys before it changes anything; if any entry fails, the table is left as it was. Two
keys that differ only in case, such as `"eur"` and `"EUR"`, must carry the same rate.
It does not require a base currency, so whatever `exportRates` returned can always be
imported again.

## Rounding and formatting

Amounts are kept at `Money`'s five decimal places. Conversions and scaling are `Money`
arithmetic, so they round as `Money` does (`mfb man money`). Use
`currency::round` to book an amount to its currency's minor units:

| Code | Minor units | `round` of `2.345` |
|---|---|---|
| `USD`, `EUR`, most currencies | 2 | `2.35` (or `2.34` under banker's rounding) |
| `JPY`, `KRW` | 0 | `2` |
| `BHD`, `KWD` | 3 | `2.345` |
| `XAU`, `XDR`, every `_` code | none: kept at 5 | `2.345` |

`currency::round(a)` uses `money::round`, so a halfway value follows the calling
thread's `money::Rounding` mode. `currency::format(a)` shows the value at the
currency's minor units, then the code (`12.50 USD`, `1500 JPY`, `1.23456 _GOLD`). Its
display rounding is `toString(Money)`'s fixed half-away-from-zero rule, whatever the
mode, so the same value always formats the same way.

## Errors

The four currency failures use generator-9 codes in subsystem `9-317`, exported as
constants so a caller can `MATCH` on them:

| Constant | Code | Raised when |
|---|---|---|
| `currency::ErrorInvalidCode` | `93170001` | A code is not an ISO 4217 code or a valid `_` code; or `importRates` names one code twice with different rates |
| `currency::ErrorInvalidRate` | `93170002` | A rate is zero or negative |
| `currency::ErrorUnknownCurrency` | `93170003` | A code has no rate in the table |
| `currency::ErrorNoBaseCurrency` | `93170004` | Two different currencies meet and no code has rate `1` |

`Money`'s own failures keep their registry codes: an out-of-range result is
`errorCode::ErrOverflow`, and dividing by zero or rounding to more than five places is
`errorCode::ErrInvalidArgument`.

```
LET total AS currency::Amount = currency::add(a, b) TRAP(e)
  IF e.code = currency::ErrorUnknownCurrency THEN RECOVER a
  FAIL e
END TRAP
```

## Testing

```
mfb test packages/currency                   # the package's own suite
packages/currency/runtime-smoke.sh           # links it into a program, checks threads
packages/currency/check-doc-examples.sh      # builds, runs and checks every DOC example
sh scripts/check-generated.sh                # data.mfb matches the vendored ISO list
```

`check-doc-examples.sh` compares each example's output with the trailing comments on its
`io::print` lines. Every rate in the suite is exact in binary `Fixed`, so each expected
value is an exact decimal worked out by hand next to its test.
