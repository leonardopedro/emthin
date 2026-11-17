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

### `export_ascii` / `import_ascii`

Write the document's ASCII-only Typst interchange projection, or replace
the document from one. `export_ascii` answers with `ascii_exported`.
`import_ascii` accepts only the ASCII projection; a UTF-8 file is
refused with `not_ascii` rather than silently accepted as if it were the
interchange form. Filesystem failures are `io_failed`.

The projection is the *rendered* document, not a second editable source:
hidden markers and `\name(...)` statements do not survive the export, so
an import replaces them with the visible text.

```json
{"jsonrpc":"2.0","method":"export_ascii","params":{"path":"/tmp/notes-ascii.typ"}}
{"jsonrpc":"2.0","method":"import_ascii","params":{"path":"/tmp/notes-ascii.typ"}}
```

### `set_text_size`

Set the document's **reading size** in points. This is a Typst reflow,
not a view zoom: larger text takes more lines and can add pages, while
the page stays fitted to the window. The value is clamped to `[8, 32]`;
any finite value is accepted, and the size that actually took effect is
reported in `text_size_changed`.

```json
{"jsonrpc":"2.0","method":"set_text_size","params":{"pt":24}}
```

### `inject_touch`

Feed one synthetic touch contact into the live gesture handlers. This is a
testing seam for the live harness: XTEST and the virtual-keyboard protocols
cannot synthesize multitouch, so a two-finger pan is delivered as a sequence
of `down` / `motion` / `up` contacts on two slots. It calls the same handlers
as real touch events, so it exercises the production pan gesture rather than
a parallel implementation. `x`/`y` are output-local logical pixels.

```json
{"jsonrpc":"2.0","method":"inject_touch","params":{"phase":"down","slot":0,"x":450,"y":420}}
{"jsonrpc":"2.0","method":"inject_touch","params":{"phase":"motion","slot":1,"x":550,"y":340}}
{"jsonrpc":"2.0","method":"inject_touch","params":{"phase":"up","slot":0,"x":450,"y":420}}
```

### `collab_version`

Ask for the document's current Loro version token. Answers with
`collab_version`, whose `version` is hex encoded. A remote peer sends that
token in `collab_join`; it is opaque and must not be parsed by the client.

```json
{"jsonrpc":"2.0","method":"collab_version","params":{}}
```

### `collab_join`

Add a collaborating peer and answer with the delta it is missing. `version`
is the peer's hex-encoded version token, or `""` for a new peer that needs
the whole document. The answer is `collab_delta`; `seq` is the first frame
on that peer's reliable stream. Calling it again for a peer that is already
joined is a reconnect: the peer's stream restarts at sequence 1 and the
delta is calculated from the supplied token.

```json
{"jsonrpc":"2.0","method":"collab_join","params":{"peer":"alice","version":""}}
```

### `collab_delta`

Apply one delta frame from a peer. `delta` is the hex-encoded Loro delta and
`seq` is that peer's reliable-stream sequence number. Missing sequence numbers
are buffered; a duplicate is ignored. Applying a delta may answer with
`collab_delta` notifications for the other peers.

```json
{"jsonrpc":"2.0","method":"collab_delta","params":{"peer":"alice","seq":1,"delta":"0a0b"}}
```

### `collab_ack`

Acknowledge every frame through `seq` for a peer. This releases the retained
frames; an acknowledgement beyond what was sent is clamped.

```json
{"jsonrpc":"2.0","method":"collab_ack","params":{"peer":"alice","seq":4}}
```

### `collab_leave`

Drop a peer and its reliable stream. No reply is emitted.

```json
{"jsonrpc":"2.0","method":"collab_leave","params":{"peer":"alice"}}
```

### `list_state`

Dump the whole document and figure list. Answers with `state`.

```json
{"jsonrpc":"2.0","method":"list_state","params":{}}
```

### `voice`

Ask the voice harness about a `\voice` dialogue block — the same turn
`Ctrl+Shift+V` queues from the keyboard. The harness (an external
`voice.sh` process, resolved from `$EMTHIN_VOICE_BIN` or `$PATH`) runs
on a worker thread, so the command returns immediately and the answer
arrives later as a `voice_result` notification (or `error`). No
`\voice` block, no harness installed, or a turn already in flight are
rejections delivered as `error`.

```json
{"jsonrpc":"2.0","method":"voice","params":{"text":"hello there"}}
{"jsonrpc":"2.0","method":"voice","params":{"block":0}}
{"jsonrpc":"2.0","method":"voice","params":{}}
```

Both fields are optional, and present-but-wrong-typed fields are
rejected rather than falling back — a client that asked for text and
silently got a *recording* (or the wrong block) is the kind of
surprise this protocol exists to avoid.

- `text`: ask this utterance. Absent: the block's pending `you:` line
  is asked if there is one; else, in a `listen`/`full` block, the
  microphone is opened and the transcription becomes the question.
- `block`: 0-based index among the document's `\voice` blocks
  (`list_state`'s `doc` shows them). Absent: the caret's block, else
  the first one — a driving script has no caret of its own.

The reply is spliced into the block's transcript as a `gemma:` line
(and the question as a `you:` line when it was not already typed
there), so the document stays the dialogue's single source of truth:
the next turn's model history is read back out of the transcript.
A failed turn splices a `voice: <reason>` line instead of vanishing.

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
| `state` | reply to `list_state` | `page`, `page_count`, `text_size`, `view_zoom`, `view_pan_x`, `view_pan_y`, `citation_style`, `citation_locale`, `doc`, `figures` |
| `doc_saved` | reply to `save_doc` | `path` |
| `ascii_exported` | reply to `export_ascii` | `path` |
| `text_size_changed` | reply to `set_text_size` | `pt`, `page_count` |
| `voice_result` | a `voice` turn finished and was spliced into its block | `block`, `utterance`, `reply` |
| `x_wayland_ready` | the XWayland socket is bound | `display` |
| `dbus_router_rule_added` | a DBus routing rule was added | `id`, `rule` |
| `dbus_router_rule_removed` | a DBus routing rule was removed | `id` |
| `dbus_router_rules` | the full routing rule list changed | `rules` |
| `collab_delta` | a peer has a delta frame to apply, or a local edit must be forwarded | `peer`, `seq`, `delta` (hex) |
| `collab_version` | reply to `collab_version` | `version` (hex) |
| `error` | a command failed | `code`, `message`, `category`, `retryable`, `remediation` — the tagged error contract, see [error-contract.md](error-contract.md) |

The three `dbus_router_*` notifications are **unsolicited**: they are emitted
from the broker's per-tick drain whenever its rule set changes, so a client
learns about a rule it did not add. `dbus_router_rules` carries the whole list
and supersedes any incremental state a client built from the other two.

A `rule` is `{id, priority, destination?, interface?, method?, target}`, where
`destination` is a glob (`None` is the wildcard) and `target` is `"host"`,
`"isolated"` or `"deny"`.

`state.text_size` is the reading size in points. `state.view_zoom` and
`state.view_pan_x` / `view_pan_y` are the presentation-only view transform.
`state.citation_style` / `citation_locale` are the open popup's overrides, or
`null` when the document's defaults apply.

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
the document's statement list.

It is a **per-pass layout address, not an identity.** It changes
whenever the document is edited *above* the statement, because that
renumbers it. `mathed_core::figures::figure_key` mints it fresh on
every render and `figures_in_frame` consumes it on the frame that same
pass produced, which is the only thing it is for.

So:

- a client that caches a key across an edit it made itself may be
  pointing at a different statement — re-`list_state`;
- a client that wants to refer to a figure *across* edits has nothing
  stable to hold. The compositor itself remembers app bindings against
  the statement's marker pair (`Figure::stable_id`), which does survive
  an insertion above, but that is not exposed on the wire. An
  `insert_above_figure` request would be the honest fix; until it
  exists, treat a key as valid only until the next edit.

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