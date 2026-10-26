# tui-logo

A LOGO turtle runner built on the `term::` TUI module. A double-line border
frames the screen, the turtle (▲, pointing the way it faces) starts at the centre, and the bottom line is
the `L>` prompt.

## Build and run

```sh
mfb build examples/tui-logo
```

- macOS / Linux — `./examples/tui-logo/build/tui-logo.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/tui-logo`,
  then run `examples/tui-logo/build/tui-logo.exe`

Run it from an interactive terminal. It opens on a title screen with the
command summary; press any key to start.

## Commands

Type a line at `L>` and press Enter. Commands are case-insensitive and are
separated by spaces (`F5` and `F 5` are the same).

| Command       | Meaning                                              |
| ------------- | ---------------------------------------------------- |
| `F [n]`       | Forward `n` spaces (default 1), drawing as it goes   |
| `B [n]`       | Backward `n` spaces (default 1), drawing as it goes  |
| `R [n]`       | Turn right `n` times, a quarter turn each (default 1) |
| `L [n]`       | Turn left `n` times, a quarter turn each (default 1)  |
| `PU`          | Pen up: the turtle moves without drawing             |
| `PD`          | Pen down: drawing resumes (the default)              |
| `TR "text"`   | Write text to the right, starting at the turtle |
| `TL "text"`   | Write text to the left, starting at the turtle: the first character is at the turtle, so it reads backwards from left to right |
| `TD "text"`   | Write text down the column, starting at the turtle |
| `TU "text"`   | Write text up the column, starting at the turtle: the first character is at the turtle, so it reads backwards from top to bottom |

Text commands never move the turtle. Text sits below the drawn path: where a
line crosses text, the line shows. The turtle is drawn on top of both.
| `FG #`        | Set the line colour, 1 to 8: black, red, green, yellow, blue, magenta, cyan, white (green by default) |
| `BG #`        | Set the background colour, 1 to 8 (none by default) |
| `S 1`         | Draw a line (the default)                            |
| `S 2`         | Draw a double line                                   |
| `S 3`         | Draw a block (█)                                     |
| `S 4`         | Draw a shade (▓)                                     |
| `C`           | Clear the drawing and send the turtle home (centre, facing north) |
| `RE n (cmds)` | Repeat `cmds` `n` times; repeats may nest            |
| `LOAD <file>` | Run the commands in a file (the rest of the line is the path) |
| `L <file>`    | Same as `LOAD`, when the word after `L` is not a number or command |
| `SAVE <file>` | Save the drawing as an ANSI art (`.ans`) file (the rest of the line is the path) |
| `Q`           | Quit                                                 |

`SAVE` writes the drawn lines cropped to their bounding box, in their colours, as UTF-8
text with CRLF line endings, so the picture can be viewed with an ANSI art
viewer. The turtle is not included. An existing file of the same name is
overwritten. The path is relative to the directory the program was started in.

A file is read from the directory the program was started in, and its
commands run as if typed on one line. A syntax error in a file is shown with
the file name. Use `LOAD` for a file whose name starts with a command letter
(such as `r.logo`) or with a digit, since `L` followed by those reads as a turn.

The turtle starts facing north. Its path is drawn with single box-drawing
glyphs, so corners and junctions meet at right angles.

Examples:

- `F 5 R F 5 R F 5 R F 5` draws a box
- `RE 4 (F 5 R)` draws the same box

## Editing the line

- **Left / Right arrows** move the insert point
- **Backspace** deletes the character before it
- Input longer than the line scrolls to keep the insert point in view
- **Enter** runs the line; a syntax error is shown in the border's title
- **Ctrl-C** quits as well as `Q`

The drawing persists across lines, so a new command continues from where the
turtle stopped, until a `C` clears it. A program that would take more than 1,000,000 steps stops
early and says so in the title. The path is not clipped to the screen: the
turtle can leave the visible area, and the drawing is shown again as soon as it
comes back. Resizing the terminal redraws everything around the same centre.

No dependencies beyond `term`, `color`, `bits`, `collections`, `errorCode`,
`io`, `strings`.
