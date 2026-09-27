# ai_chat

A multi-session Claude chat TUI, built on the `term::` double-buffered
surface and the `process::` streaming child API. Each session drives its own
`claude` CLI child process in streaming-JSON mode; children run concurrently,
so switching sessions never stalls an in-flight generation. See the header
comment in [`src/main.mfb`](src/main.mfb) for the design notes (the session
map, why the data model is resource-free).

## Build and run

```sh
mfb build examples/ai_chat
```

- macOS / Linux — `./examples/ai_chat/build/ai_chat.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/ai_chat`,
  then run `examples/ai_chat/build/ai_chat.exe`

Run it from an interactive terminal.

## Dependency: the `claude` CLI

This example spawns `claude` as a child process (`process::spawn`) — it needs
the [Claude Code CLI](https://docs.claude.com/en/docs/claude-code) already
installed and on `PATH`. There is no vendored or package-managed copy; if
`claude` is missing, spawning a turn fails.

## Controls

- **Tab** — cycle focus: Sidebar → Transcript → Input
- **Up / Down** — scroll the transcript (in Sidebar focus: switch session)
- **PageUp / PageDown** — scroll the transcript by a page
- **Enter** (Input focus) — send the prompt / run a slash command
- **Ctrl+N** — new session
- **Ctrl+X** — close the active session (asks to confirm)
- **Ctrl+C** — quit

Slash commands typed into the input line:

- `/new [message]` — new session, optionally sending a first message
- `/model` — open a picker to change the session's model
- `/close` — close this session (confirmation modal)
- `/exit` — quit
- `/help` — show the help modal

A modal (help, model picker, close-confirm) owns all input while open: Up /
Down + Enter in the picker, Y / N in the confirm, any key closes help, Esc
cancels.
