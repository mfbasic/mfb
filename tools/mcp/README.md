# MFBASIC MCP server

A [Model Context Protocol](https://modelcontextprotocol.io) server that exposes
MFBASIC's built-in help and language specification to MCP-aware assistants. It
wraps two `mfb` subcommands:

| Tool       | Backed by   | Purpose                                            |
| ---------- | ----------- | -------------------------------------------------- |
| `mfb_man`  | `mfb man`   | Built-in package & function reference.             |
| `mfb_spec` | `mfb spec`  | The embedded MFBASIC language specification.        |

## Setup

```sh
cd tools/mcp
npm install
```

The server shells out to the `mfb` binary. It is located in this order:

1. `$MFB_BIN` if set (an explicit path to the binary).
2. `target/release/mfb` then `target/debug/mfb` inside this checkout.
3. `mfb` on `$PATH`.

Build one first, e.g. `cargo build --release`.

## Tools

### `mfb_man`

| Argument   | Type     | Notes                                                    |
| ---------- | -------- | -------------------------------------------------------- |
| `package`  | string?  | Package name (`io`, `strings`, …). Omit to list all.     |
| `function` | string?  | Function/topic in the package. Requires `package`.       |

- No arguments → package index.
- `package` only → that package's function list.
- `package` + `function` → full function detail.

### `mfb_spec`

| Argument   | Type     | Notes                                                       |
| ---------- | -------- | ---------------------------------------------------------- |
| `topic`    | string?  | Spec topic (`language`, `diagnostics`, …). Omit for index. |
| `subtopic` | string?  | Section within the topic. Requires `topic`.                |
| `all`      | boolean? | Render the whole topic. Requires `topic`, excludes `subtopic`. |

## Client configuration

Add to your MCP client config (e.g. `claude_desktop_config.json` or a project
`.mcp.json`):

```json
{
  "mcpServers": {
    "mfbasic": {
      "command": "node",
      "args": ["/absolute/path/to/mfb/tools/mcp/index.js"]
    }
  }
}
```

To point at a specific `mfb` build, add an env override:

```json
{
  "mcpServers": {
    "mfbasic": {
      "command": "node",
      "args": ["/absolute/path/to/mfb/tools/mcp/index.js"],
      "env": { "MFB_BIN": "/absolute/path/to/mfb" }
    }
  }
}
```

`MFB_MCP_WIDTH` (default `100`) controls the column width used when rendering
spec output.
