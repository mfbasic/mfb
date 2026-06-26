# Tables

The reason specs render through this engine instead of plain text: tables reflow
to the terminal width. Author them as ordinary GFM pipe tables and let the
renderer size the columns. Try `mfb spec dummy tables --width 100` and again at
`--width 50` to watch the cells wrap.

## Alignment

Column alignment comes from the separator row (`:--`, `--:`, `:--:`):

| Name | Width | Notes |
| :--- | ----: | :---: |
| left | 1 | left-aligned text |
| right | 22 | right-aligned numbers |
| center | 333 | centered |

## Wide content

When the natural column widths exceed the terminal, the widest column shrinks
first and its cells wrap across multiple rows:

| Field | Type | Description |
| --- | --- | --- |
| `name` | `String` | The fully-qualified identifier, which can be quite long and will wrap to several lines on a narrow terminal. |
| `offset` | `Int` | Byte offset into the record payload, counted from zero. |
| `flags` | `Int` | A bitfield; see the encoding topic for the meaning of each bit and how they combine under masking. |

A short table stays compact:

| Key | Value |
| --- | --- |
| a | 1 |
| b | 2 |
