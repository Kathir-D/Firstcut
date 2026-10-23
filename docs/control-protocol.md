# Control protocol

`firstcut --control <port> [shoot-folder]` opens the real desktop window and a
TCP listener on `127.0.0.1`. It exists so a script or an agent can operate the
app the way a person does: every engine verb goes through the same
`registry::dispatch` the menus and keyboard use, and `ui.*` renders the live
state through the same headless harness as the snapshot tests.

```sh
firstcut --control 4317 ~/Pictures/shoot
```

The channel is newline-delimited JSON, one request object per line and one
reply object per line. A connection can carry many requests; close it when
done. The listener is localhost-only and has no authentication — treat it as a
developer tool, not a remote API.

## Request

```json
{"id": 1, "method": "engine.execute", "params": {"id": "rate.stars", "params": {"stars": 4}}}
```

- `id` is any JSON value and is echoed in the reply (`null` if absent).
- `method` is one of the names below.
- `params` is method-specific and defaults to `{}`.

## Reply

```json
{"id": 1, "result": {"stars": 4}}
```

or

```json
{"id": 1, "error": "rate.stars: stars is 0..=5"}
```

An unparseable line answers `{"error":"unparseable request line"}` and closes
the connection. An unknown method answers with an error and keeps the
connection open. A request that the UI thread does not answer within 10 seconds
returns `"the app did not answer within 10 s — is the window alive?"`; the
request may still have run, so read `engine.state` before retrying a mutation.

## Methods

| Method | Params | Result |
| --- | --- | --- |
| `engine.execute` | `{"id": "<registry id>", "params": {…}}` | the command's JSON result |
| `engine.state` | none | the registry's state snapshot |
| `engine.commands` | none | every command id, description and parameter |
| `ui.widgets` | none | `{"widgets":[{"name","role","rect":[x,y,w,h]}]}` for named accessibility nodes |
| `ui.clickWidget` | `{"name":"<accessible name>"}` | the widget tree after the click; an error if no node has that name |
| `ui.screenshot` | none | `{"path":"/tmp/firstcut-screenshot-….png"}` on the app's machine |

`engine.state` and `engine.commands` are themselves registry commands
(`engine.state`, `engine.commands`), so their result is exactly what the CLI
shows. `ui.clickWidget` matches the same accessible names `ui.widgets` lists;
for example `filmstrip: IMG_0003.CR3` or `Rate 4 stars`.

## Example

```sh
python3 - <<'PY'
import json, socket
s = socket.create_connection(("127.0.0.1", 4317))
def call(n, method, params=None):
    s.sendall((json.dumps({"id": n, "method": method, "params": params or {}}) + "\n").encode())
    return json.loads(s.makefile("r").readline())
print(call(1, "engine.execute", {"id": "file.open", "params": {"path": "/tmp/shoot"}}))
print(call(2, "engine.state"))
print(call(3, "ui.widgets"))
print(call(4, "ui.clickWidget", {"name": "Rate 4 stars"}))
print(call(5, "ui.screenshot"))
PY
```

## Notes

- Requests are answered on the UI thread inside the next frame. The developer
  window repaints continuously by default, and a queued request explicitly wakes
  it, so an idle window still answers.
- `ui.screenshot` writes a PNG under the platform temporary directory and
  returns its path; the file stays until the OS cleans it up.
- The channel supports one shoot per process. Use the engine's `file.open`
  command to switch folders rather than restarting unless you need a clean
  process.
- The same verbs are available headlessly over MCP (`firstcut-cli mcp`) and the
  CLI (`firstcut-cli run`); see [mcp.md](mcp.md) and the CLI usage.
