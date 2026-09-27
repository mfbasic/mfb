# hangman

A small terminal word-guessing game built on the `term::` TUI module: pick a
letter, get closer to the word or add a piece to the gallows.

## Build and run

```sh
mfb build examples/hangman
```

- macOS / Linux — `./examples/hangman/build/hangman.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/hangman`,
  then run `examples/hangman/build/hangman.exe`

Run it from an interactive terminal — it switches to the alternate screen and
reads single keys without echo.

## Controls

- **A–Z** — guess a letter
- **Q** — quit (at the guess prompt, or at the win/loss prompt)
- any other key at the win/loss prompt — play again with a new word

Six wrong guesses ends the round; the word is picked at random from a fixed
word list built into the program.

No dependencies beyond `term`, `color`, `strings`, `math`, `collections`, `io`.
