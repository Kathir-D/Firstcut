# MCP server

`firstcut-cli mcp` exposes the command registry as MCP tools over stdio. There is
no second list to maintain: `tools/list` is generated from
`firstcut_engine::registry::REGISTRY`, and `tools/call` goes through the same
`dispatch` as the menus, the keyboard and the CLI.

```sh
firstcut-cli mcp
```

stdout is the protocol (newline-delimited JSON-RPC 2.0); logs and diagnostics go
to stderr. The process serves one in-memory `Model`; `file.open` is normally the
first call, and every later call sees that model's state.

## Methods

| Method | Behaviour |
| --- | --- |
| `initialize` | echoes the client's `protocolVersion` (default `2025-06-18`), declares the `tools` capability and reports `serverInfo` |
| `ping` | `{}` |
| `tools/list` | one tool per registry entry |
| `tools/call` | dispatches `{name, arguments}` through the registry |

Notifications (requests without an `id`) are not answered. Unknown requests get
JSON-RPC error `-32601`; malformed JSON gets `-32700` and the server keeps going.

## Tool names

A registry id has one dot and no underscores, so the tool name is
`firstcut_<id with dots replaced by underscores>`:

| Registry id | MCP tool |
| --- | --- |
| `file.open` | `firstcut_file_open` |
| `rate.stars` | `firstcut_rate_stars` |
| `settings.autoAdvance` | `firstcut_settings_autoAdvance` |

Both forms are accepted in `tools/call`: the prefixed name and the raw dotted id
(`{"name": "rate.stars", …}`). The tool description carries the registry id, so
a client that only shows descriptions still exposes the canonical verb.

## Input schemas

The schema is read from the registry's declared parameters. A parameter whose
kind contains `uint` is an integer with `minimum: 0`; every other parameter is a
string. All declared parameters are required.

```json
{
  "name": "firstcut_rate_stars",
  "description": "Rate the current photo with 0–5 stars (registry id: rate.stars)",
  "inputSchema": {
    "type": "object",
    "properties": {"stars": {"type": "integer", "minimum": 0}},
    "required": ["stars"]
  }
}
```

## Results

A call returns MCP content, not a protocol error:

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "content": [{"type": "text", "text": "{\n  \"stars\": 4\n}"}],
    "isError": false
  }
}
```

A rejected command (`stars` out of range, a folder with no photos, a bad
parameter) comes back with `"isError": true` and the typed error text. That is
deliberate: the model is still usable after a bad call, so the client can read
`engine.state` and correct it.

## Example session

```text
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"firstcut_file_open","arguments":{"path":"/tmp/shoot"}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"firstcut_rate_stars","arguments":{"stars":4}}}
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"firstcut_engine_state","arguments":{}}}
```

The CLI's `--connect host:port` bridge to a running window is **not implemented
yet**; this server owns its `Model` in-process. To drive the real window, use the
[control protocol](control-protocol.md).
