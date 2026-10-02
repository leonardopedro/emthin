# emthin control protocol (IPC)

JSON-RPC 2.0 over a Unix socket, `Content-Length` framing. emthin
speaks it as a **notification-only** protocol: the compositor pushes
events and accepts commands; there are no request/response pairs and no
`id` field. A client that wants an answer to a command asks for it by
issuing `list_state` and waiting for the next `state` event.

The socket path defaults to `$XDG_RUNTIME_DIR/emthin-<pid>.ipc` and can
be pinned with `--ipc-path`. Exactly one client may be connected at a
time; a second connection is accepted only after the first drops.

This replaced the Emacs-driven protocol, whose only job was to let an
Elisp layout engine tell the compositor where to put windows. The
compositor now lays out its own document, so the protocol is an
*observation and command* surface: enough to drive it from outside
without re-deriving its state.

## Coordinate space

Every rectangle in the protocol is **output-local logical pixels** —
the same space `emthin` renders into, origin at the window's top-left,
already divided by the output's scale. Not fractions, not device
pixels: the document decides geometry in Typst points, and the control
plane reports what actually got placed.

## Client → compositor

### `spawn`

Launch a program. Its toplevel binds to a figure — an unbound `\app`
whose id matches the program's `app_id` or title (glob), else the
first unbound figure, else a **new figure appended to the document**.

```json
{"jsonrpc":"2.0","method":"spawn","params":{"cmd":"foot","args":["-T","xterm-256color"]}}
```

`args` is optional.

### `close`

Close the app bound to a figure. The `\app` statement stays — the
figure goes dormant and can be relaunched.

```json
{"jsonrpc":"2.0","method":"close","params":{"figure":"f0"}}
```

### `focus`

Move keyboard focus. `figure: null` (or absent) returns focus to the
document's caret.

```json
{"jsonrpc":"2.0","method":"focus","params":{"figure":"f0"}}
```

### `set_figure_size`

Rewrite a figure's `\app` width/height arguments. Both are clamped to
`[64, 8192]`. A no-op change is not written and emits nothing.

```json
{"jsonrpc":"2.0","method":"set_figure_size","params":{"figure":"f0","w":800,"h":600}}
```

### `clone_figure`

Duplicate a figure's caption + statement (with fresh marker ids) at the
end of the document. The two figures share a binding id, so the same
app renders in both.

```json
{"jsonrpc":"2.0","method":"clone_figure","params":{"figure":"f0"}}
```

### `goto_page`

Show a document page. 0-based. Out-of-range pages are rejected with an
`error` event.

```json
{"jsonrpc":"2.0","method":"goto_page","params":{"page":2}}
```

### `open_doc` / `save_doc`

Replace the document from a file, or snapshot the current one to a
file. `save_doc` answers with `doc_saved`.

```json
{"jsonrpc":"2.0","method":"open_doc","params":{"path":"/tmp/notes.typ"}}
{"jsonrpc":"2.0","method":"save_doc","params":{"path":"/tmp/notes.typ"}}
```

### `list_state`

Dump the whole document and figure list. Answers with `state`.

```json
{"jsonrpc":"2.0","method":"list_state","params":{}}
```

### `dbus_router_add_rule` / `dbus_router_remove_rule` / `dbus_router_list_rules`

DBus routing rules for embedded clients (unchanged from the pre-rewrite
protocol).

```json
{"jsonrpc":"2.0","method":"dbus_router_list_rules","params":{}}
```

## Compositor → client

| `method` | When | `params` |
|---|---|---|
| `connected` | on connect | `version` |
| `figure_changed` | a figure changed shape, page or binding | `figure`, `page`, `rect`, `bound` |
| `figure_bound` | an app mapped into a figure | `figure`, `window_id`, `title` |
| `figure_unbound` | an app left a figure (closed or exited) | `figure`, `window_id` |
| `app_title_changed` | a bound app's title changed | `window_id`, `title` |
| `page_changed` | the visible page changed | `page` |
| `state` | reply to `list_state` | `page`, `page_count`, `doc`, `figures` |
| `doc_saved` | reply to `save_doc` | `path` |
| `x_wayland_ready` | the XWayland socket is bound | `display` |
| `error` | a command failed | `message` |

`state.figures` is a list of:

```json
{
  "key": "f0",
  "id": "term",
  "caption": "Terminal demo",
  "rect": {"x": 300, "y": 50, "w": 640, "h": 400},
  "page": 0,
  "window_id": 7,
  "title": "foot"
}
```

`window_id` and `title` are `null` for a dormant figure. `rect` is
zero-sized for a figure that isn't on the visible page.

## Figure keys

A figure's `key` is `f<stmt-index>` — the `\app` statement's index in
the document's statement list. It is **stable for the lifetime of the
statement**: it does not change when the document is edited elsewhere,
and it disappears only when the statement itself is deleted.

Keys are *not* revision-stamped. A client that caches a key across
edits will keep pointing at the same figure; a client that wants to be
sure should re-`list_state` after an edit it made itself.

## Ext-workspace-v1

Pages are also exported over `ext_workspace_v1`, one workspace per
page, so an external bar can list and switch them. Workspace ids are
the page index **plus one** (protocol ids are non-zero by
specification). `Remove` is rejected: pages come from the document's own
pagination and are not independently creatable or destructible.

## External-shell mode (planned)

The figure-rect reporting above is exactly what a remote frontend would
need to render the document itself. An `--external-shell` mode where
emthin skips in-process document rendering and accepts figure rects
over IPC is designed-for but **not implemented** — see
`docs/REWRITE_PLAN.md` §5.13.

## Example

```sh
SOCK=$(ls -t "$XDG_RUNTIME_DIR"/emthin-*.ipc | head -1)

say() { printf 'Content-Length: %d\r\n\r\n%s' "${#1}" "$1"; }

say '{"jsonrpc":"2.0","method":"list_state","params":{}}'
# → {"jsonrpc":"2.0","method":"state","params":{"page":0,...}}

say '{"jsonrpc":"2.0","method":"set_figure_size","params":{"figure":"f0","w":800,"h":600}}'
# → {"jsonrpc":"2.0","method":"figure_changed","params":{"figure":"f0",...}}
```