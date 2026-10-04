# money-demo

A terminal simulation of one person's finances, built to show what the `Money`
type does that `Float` cannot. Days tick by: paychecks split into checking,
savings, a 401(k) and tax withholding; rent, utilities, fuel, groceries, dining
and subscriptions come out of checking; savings earns interest, the 401(k)
rides a random market, and every April the previous year's taxes are filed.
Every two or three years an emergency (a car crash, an ER visit, a flooded
basement...) is paid from savings. Every year or two something (a vacation, a
car, a computer) is saved up for in equal monthly installments whose last one
absorbs the rounding remainder, so they add up to the goal to the cent, and
then bought. The simulation stops after a 40-year career.

Every amount is computed at Money's five decimal places and booked to whole
cents with `money::round` - the log line under the columns shows both (fuel at
`$3.479` a gallon, a 2.25% tax, interest at 4.25% APY divided by twelve).

The only stored state is the event log, each event holding its raw inputs.
Balances and columns are derived by posting the log into a ledger, so:

- **r** switches `money::Rounding` between Commercial and Banker, replays the
  whole history under the new mode, and shows exactly which entries, booked
  amounts and balances moved.
- **f** switches the view to a second ledger built from the same log with
  binary `Float` arithmetic, and shows where the two disagree - amounts with no
  exact binary form, raw results that differ, cents that book differently, and
  how far each Float balance has drifted.

## Build and run

```sh
mfb build examples/money-demo
```

- macOS / Linux — `./examples/money-demo/build/money-demo.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/money-demo`,
  then run `examples/money-demo/build/money-demo.exe`

Run it from an interactive terminal of at least 80x25. A wider terminal centres
the frame; a taller one shows more entries in each column.

## Controls

- **Space** — pause / resume
- **+** / **-** — faster / slower (1 to 120 simulated days per second)
- **R** — switch Commercial / Banker rounding
- **F** — switch the Money / Float view
- **Q** — quit (any key closes an open report)

No dependencies beyond `term`, `color`, `money`, `datetime`, `math`, `strings`,
`collections`, `errorCode`, `io`.
