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

Replace the document from a file, or write the current one to a file.
`open_doc` detects a raw Loro snapshot by content first, then UTF-8 source.
Opening a snapshot restores text, marks and statement structure; opening source
creates a fresh document. `save_doc` answers with `doc_saved` containing the
path, the actual `format` (`source` or `snapshot`) and the SHA-256 `digest` of
the bytes written.

`format` is optional; absent, `.loro` and `.mathed` paths save a snapshot and
all other paths save source. `force` is optional and defaults to `false`.
Without it, a target that changed since it was opened or last saved is refused
with `doc_conflict`; `force: true` replaces it. A destination this document has
never opened or saved is also refused unless `force` is set, and a newly
created path is published without replacing a file another process created in
the meantime. Saving through a symlink writes and stamps its resolved target
while keeping the link; retargeting that link is a conflict.

The replacement is atomic on a local filesystem. The final check cannot stop a
separate process that writes the same existing target between that check and
the rename; use `force` deliberately and do not treat this as mutual exclusion.

```json
{"jsonrpc":"2.0","method":"open_doc","params":{"path":"/tmp/notes.typ"}}
{"jsonrpc":"2.0","method":"save_doc","params":{"path":"/tmp/notes.typ"}}
{"jsonrpc":"2.0","method":"save_doc","params":{"path":"/tmp/notes.mathed","format":"snapshot","force":true}}
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
on that peer's reliable stream and `epoch` is the connection epoch assigned to
this peer, which it must echo on the frames it sends back. Calling it again for
a peer that is already joined is a reconnect: the peer's stream restarts at
sequence 1, a fresh `epoch` is assigned (frames stamped with the old one are
refused) and the delta is calculated from the supplied token. The optional
`scope` is the caller's document generation; a scope-aware peer learns this
host's generation from the response and echoes it on later frames. Omitting it
keeps the legacy unscoped path.

```json
{"jsonrpc":"2.0","method":"collab_join","params":{"peer":"alice","version":"","scope":1}}
```

### `collab_delta`

Apply one delta frame from a peer. `delta` is the hex-encoded Loro delta and
`seq` is that peer's reliable-stream sequence number. Missing sequence numbers
are buffered; a duplicate is ignored. Applying a delta may answer with
`collab_delta` notifications for the other peers. `scope` echoes the receiving
host's document generation; a mismatch is stale traffic from a replaced
document and is ignored. Scope-aware peers must include it; omitting it keeps
the legacy path and is accepted only for a legacy join. `epoch` is the
connection epoch this peer learned from its `collab_join` reply: a frame from a
replaced connection, or one whose epoch a `collab_revoke` invalidated, is
refused before it touches the document. On a share with explicit grants the
session itself checks the grant, so a read-only peer cannot commit. Omitting
`epoch` keeps the unfenced legacy path.

```json
{"jsonrpc":"2.0","method":"collab_delta","params":{"peer":"alice","seq":1,"delta":"0a0b","scope":1}}
```

### `collab_ack`

Acknowledge every frame through `seq` for a peer. This releases the retained
frames; an acknowledgement beyond what was sent is clamped. `epoch` is the
connection epoch this peer learned from its `collab_join` reply: an
acknowledgement stamped with a replaced epoch is refused as `collab_failed`, so
it cannot release frames for a connection that no longer exists. Omitting
`epoch` keeps the unfenced legacy path.

```json
{"jsonrpc":"2.0","method":"collab_ack","params":{"peer":"alice","seq":4,"epoch":2}}
```

### `collab_leave`

Drop a peer and its reliable stream. No reply is emitted.

```json
{"jsonrpc":"2.0","method":"collab_leave","params":{"peer":"alice"}}
```

### `collab_presence`

Apply one peer's presence payload. `peer` is the transmitting peer and `blob`
is the hex-encoded payload produced by another instance's `collab_presence`
notification. Presence is advisory: it never edits the document, revision or
undo history. An expired or malformed payload is rejected as `collab_failed`.
The optional `scope` is the document generation from the sender's
`collab_presence` notification; a stale generation is ignored.

```json
{"jsonrpc":"2.0","method":"collab_presence","params":{"peer":"alice","blob":"0a0b","scope":1}}
```

### `collab_name`

Set this process's collaboration display name and publish it immediately. The
name defaults to a short stable peer id; there is no account system.

```json
{"jsonrpc":"2.0","method":"collab_name","params":{"name":"Ada"}}
```

### `collab_shares`

Ask whether `peer` may discover this share. The answer is a
`collab_share_access` notification with `visible: false` for both a denied and
an unknown identity, so a client cannot enumerate shares.

```json
{"jsonrpc":"2.0","method":"collab_shares","params":{"peer":"bob"}}
```

### `collab_grant` / `collab_revoke`

Grant `read`, `edit` or `invite` on this share to `peer`, or revoke all its
grants and invalidate its pending connection work. Only the verified owner may
call them; a display name in presence carries no authority. Once any grant is
made, every non-owner join, delta and presence is checked against the owner's
grant table. An unconfigured share keeps the legacy unscoped path.

```json
{"jsonrpc":"2.0","method":"collab_grant","params":{"peer":"bob","grant":"edit"}}
{"jsonrpc":"2.0","method":"collab_revoke","params":{"peer":"bob"}}
```

### `collab_anchor`

Apply a share-scoped, anchored collaborator caret. `blob` is hex encoded and
`epoch` (optional) fences a stale connection. An anchor is resolved only in a
compatible document view; a caret whose source is missing is hidden. Presence
never edits the document, undo history or saved file.

```json
{"jsonrpc":"2.0","method":"collab_anchor","params":{"peer":"bob","blob":"0a0b","epoch":1}}
```

### `history_list` / `history_preview` / `history_export`

Read-only local document history. `history_list` answers with `history`, the
retained frontiers (hex, oldest first). `history_preview` answers with a
`history_preview` carrying the share id, owner, text, marks and content digest
at that frontier, or `status: "unavailable"` with a reason when the frontier
was compacted or belongs to another share. A preview is read-only: it changes
neither the live document, its save baselines nor its undo stack.
`history_export` is the only explicit copy-export path — it makes the retained
frontier a new local document (and resets the collaboration scope).

```json
{"jsonrpc":"2.0","method":"history_list","params":{}}
{"jsonrpc":"2.0","method":"history_preview","params":{"frontier":"0a0b"}}
{"jsonrpc":"2.0","method":"history_export","params":{"frontier":"0a0b"}}
```

### `inspect_start` / `inspect_stop` / `inspect_state`

Opt-in bounded session diagnostics (K7). `inspect_start` enables a 512-event
metadata buffer; `inspect_stop` disables it and drops the buffer — observation
creates no file, timer or event queue of its own. Events correlate by run,
share id, peer, frontier and connection epoch and never carry note text,
terminal bytes, presence blobs or keys. `inspect_state` answers with
`inspect_state`, the settled state computed by the same reducer an NDJSON
replay uses, so a fixture and a live session show the same result.

```json
{"jsonrpc":"2.0","method":"inspect_start","params":{}}
{"jsonrpc":"2.0","method":"inspect_state","params":{}}
{"jsonrpc":"2.0","method":"inspect_stop","params":{}}
```

### `semantic_outline` / `goto_definition` / `rename_definition`

Read-only semantic navigation over mathed's existing `SemanticIndex` (parked
item). `semantic_outline` answers with `semantic_outline`: definitions with use
counts, resolved uses and unresolved names, all with `[start,end]` UTF-8 byte
ranges. `goto_definition` moves the caret to the definition covering a byte (a
resolved use also resolves to its definition); a missing target returns
`not_found`. `rename_definition` replaces the definition's name and every
resolved use as one checked grouped edit, answering with `rename_result` and
the number of ranges changed; one undo step reverses it.

```json
{"jsonrpc":"2.0","method":"semantic_outline","params":{}}
{"jsonrpc":"2.0","method":"goto_definition","params":{"byte":42}}
{"jsonrpc":"2.0","method":"rename_definition","params":{"def_index":0,"new_name":"kind"}}
```

### `proof_search`

A second, independent query over the same proof facts (K9). `proof_search`
filters definitions, references and unresolved names by case-insensitive name
substring, sorts each bucket by source offset, and bounds each to `limit`. It
returns `proof_matches` as `(name,start,end)` rows so a client can key a
keyed/incremental view by the returned ranges; it never resplices or mutates
the document. A search before any `proof_query` builds the view itself.

```json
{"jsonrpc":"2.0","method":"proof_search","params":{"query":"norm","limit":8}}
```

### `client_identity`

Read the kernel-verified local control-client credentials (K4). `SO_PEERCRED`
is captured by the kernel when the Unix socket connection is accepted, so a
client cannot claim another process's pid/uid/gid in a message field. The reply
is `client_identity` with `pid`, `uid` and `gid` (all null when the platform
cannot report them). This is the local-principal fact the admission layer can
use; it is **not** by itself an authorization decision — the live endpoint still
verifies remote certificates and capabilities independently.

```json
{"jsonrpc":"2.0","method":"client_identity","params":{}}
```

### `lint_run` / `plugin_catalog`

The helper-process plugin framework (parked item). `lint_run` queues one
`\lint` diagnostics pass through the generic one-in-flight helper primitive;
a missing helper is a normal no-op. Diagnostics are derived annotation state
spliced into the render text, never written into the Loro document.
`plugin_catalog` lists the three plugins (`formal`, `voice`, `diagnostics`)
with their availability and run counts, making the framework observable
without a separate registry.

```json
{"jsonrpc":"2.0","method":"lint_run","params":{}}
{"jsonrpc":"2.0","method":"plugin_catalog","params":{}}
```

### `collab_publish_descriptor` / `collab_descriptor`

Optional AT Protocol session-descriptor publication (parked item).
`collab_publish_descriptor` builds a signed discovery record from the current
document revision and an owner DID / joinable endpoint, and publishes it to the
local PDS adapter, answering with `collab_descriptor_published`. The record
carries **no deltas, snapshots, raw version tokens, document text or presence**
— it says only which document is shared, who owns it, and where the live
`emthin-collab` endpoint is. Publication is best-effort: a PDS outage is
reported but never touches the edit or save path. `collab_descriptor` fetches
a record for discovery only; the live `Hello` handshake still decides the real
missing delta, and the live endpoint — not the descriptor — authorizes joins.

```json
{"jsonrpc":"2.0","method":"collab_publish_descriptor","params":{"owner":"did:web:notes.example.org","endpoint":"wss://notes.example.org/emthin/session/7Q2","rkey":"3KQ"}}
{"jsonrpc":"2.0","method":"collab_descriptor","params":{"rkey":"3KQ"}}
```

### `kernel_run` / `kernel_outputs`

Notebook/kernel execution (parked item). `kernel_run` queues one block for an
external kernel binary (pinned by `EMTHIN_KERNEL_BIN`) on a worker thread,
answering with `kernel_submitted` and a generation fence. A second run while
one is in flight is rejected — one owner per async job. Output is **derived
state**: it is spliced into the render text through Typst block splices and
never written into the Loro document, so a failed or stale run cannot lose
data. `kernel_outputs` reads the last-good regions as `[block_end, markup]`
pairs; a failure keeps the previous output for that block, and a document edit
invalidates regions at or after the change.

```json
{"jsonrpc":"2.0","method":"kernel_run","params":{"block_end":412,"code":"1+1","lang":"python"}}
{"jsonrpc":"2.0","method":"kernel_outputs","params":{}}
```

### `launcher_entries` / `launcher_launch`

The app launcher palette (parked item). `launcher_entries` answers with
`launcher_list`: the dormant figures whose `\app` statement carries a `launch:`
command, as `(figure key, label, page)` triples. It keeps **no command
registry** — every row comes from the document. `launcher_launch` runs one by
key through the same `relaunch_dormant` path a click/Return uses, answering with
`launcher_launched` and whether a spawn was attempted. A figure whose `launch:`
changed between listing and launching is interpreted from the document at
launch time, never from a cached command string.

```json
{"jsonrpc":"2.0","method":"launcher_entries","params":{}}
{"jsonrpc":"2.0","method":"launcher_launch","params":{"figure":"f0"}}
```

### `a11y_nodes`

Read-only rendered-text accessibility nodes (parked item). Because the
compositor disables `mathed_mini`'s `gui` feature, the nodes come from
`mathed_core::accessibility::build_access_nodes`, which is always available.
Each node is `{role,label,value,start,end}` with UTF-8 byte ranges (`start`/
`end` null when the node has no source span). This is the testable desktop
authority a platform AT bridge maps to a native tree; the query itself calls
no platform API.

```json
{"jsonrpc":"2.0","method":"a11y_nodes","params":{}}
```

### `proof_query` / `proof_result`

Read-only proof/reference queries (K8). A `\formal` statement may cite an
external verifier record with a named `proof: "id"` argument; that id, rather
than the layout-addressed `g<stmt>` key, becomes the `proof_id` a matching
`proof_result` must carry (see `scripts/e2e/live-mm0.sh` for a real mm0web run).
`proof_query` answers with `proof_view`:
definitions and references from mathed's existing semantic index, missing
targets, and the last accepted proof result with explicit ownership. `proof_result`
delivers an independently fetched verdict with its `share_id`, `document_key`
and `proof_id`; a late result for another proof or document is ignored and
never changes the document, its save baselines or its undo history.

```json
{"jsonrpc":"2.0","method":"proof_query","params":{}}
{"jsonrpc":"2.0","method":"proof_result","params":{"share_id":"01","document_key":"proof.typ","proof_id":"f0","verdict":"verified"}}
```

### `list_state`

Dump the whole document, figure list and live peer snapshot. Answers with
`state`.

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
| `doc_changed` | one grouped, committed document edit | `seq`, `document`, `content_generation`, `version` (hex Loro version vector), `origin` (`local`/`peer`/`undo`/`redo`/`restore`), `scope` (`content`/`presentation`), `ranges` (`[start,end]` byte pairs), `source_digest`, `content_digest` |
| `state` | reply to `list_state` | `page`, `page_count`, `text_size`, `view_zoom`, `view_pan_x`, `view_pan_y`, `citation_style`, `citation_locale`, `doc`, `figures`, `peers`, `collab_local_peer`, `collab_local_name`, `status`, `save_state`, `recovery_state`, `preview_state`, `layout_error`, `save_error`, `recovery_error`, `save_warning`, `local_state`, `share_state`, `local_state_detail`, `share_state_detail`, `seed_granted`, `last_recovery_ack`, `last_explicit_ack`, `last_peer_ack` |
| `doc_saved` | reply to `save_doc` | `path` |
| `ascii_exported` | reply to `export_ascii` | `path` |
| `text_size_changed` | reply to `set_text_size` | `pt`, `page_count` |
| `voice_result` | a `voice` turn finished and was spliced into its block | `block`, `utterance`, `reply` |
| `x_wayland_ready` | the XWayland socket is bound | `display` |
| `dbus_router_rule_added` | a DBus routing rule was added | `id`, `rule` |
| `dbus_router_rule_removed` | a DBus routing rule was removed | `id` |
| `dbus_router_rules` | the full routing rule list changed | `rules` |
| `collab_delta` | a peer has a delta frame to apply, or a local edit must be forwarded | `peer`, `seq`, `delta` (hex), `scope` (null for legacy) |
| `collab_version` | reply to `collab_version` | `version` (hex) |
| `collab_presence` | the local heartbeat/join published a presence blob, or a remote peer snapshot changed | `local_peer`, `local_name`, `peers`, `blob` (hex or `null`), `scope` |
| `collab_share_access` | reply to `collab_shares` | `peer`, `visible` |
| `semantic_outline` | reply to `semantic_outline` | `definitions` (`[index,name,uses,start,end]`), `uses` (`[name,start,end,definition]`), `unresolved` (`[name,start,end]`) |
| `rename_result` | reply to `rename_definition` | `replacements`, `applied` |
| `proof_matches` | reply to `proof_search` | `definitions`/`references` (`[name,start,end]`), `missing` |
| `client_identity` | reply to `client_identity` | `pid`, `uid`, `gid` (kernel-verified, nullable) |
| `lint_result` | reply to `lint_run` | `generation`, `accepted` |
| `plugin_catalog` | reply to `plugin_catalog` | `plugins` (`[name,available,runs]`) |
| `collab_descriptor_published` | reply to `collab_publish_descriptor` | `rkey`, `revision` |
| `collab_descriptor` | reply to `collab_descriptor` | `json` (null when absent) |
| `kernel_submitted` | reply to `kernel_run` | `generation`, `accepted` |
| `kernel_outputs` | reply to `kernel_outputs` | `regions` (`[block_end,markup]`) |
| `launcher_list` | reply to `launcher_entries` | `entries` (`[figure,label,page]`) |
| `launcher_launched` | reply to `launcher_launch` | `figure`, `spawned` |
| `a11y_nodes` | reply to `a11y_nodes` | `nodes` (`[role,label,value,start,end]`) |
| `proof_view` | reply to `proof_query` | `share_id`, `document_key`, `revision`, `definitions` (`[name,start,end,uses]`), `references` (`[name,start,end,target]`), `missing`, `proof_id`, `verdict` |
| `inspect_state` | reply to `inspect_state` | `enabled`, `local`, `share`, `frontier`, `accepted`, `peers`, `reason` |
| `history` | reply to `history_list` | `frontiers` (hex) |
| `history_preview` | reply to `history_preview` | `frontier`, `share_id`, `owner`, `seq`, `text`, `marks` (`[start,end,key]`), `content_digest`, `status`, `reason` |
| `error` | a command failed | `code`, `message`, `category`, `retryable`, `remediation` — the tagged error contract, see [error-contract.md](error-contract.md) |

The three `dbus_router_*` notifications are **unsolicited**: they are emitted
from the broker's per-tick drain whenever its rule set changes, so a client
learns about a rule it did not add. `dbus_router_rules` carries the whole list
and supersedes any incremental state a client built from the other two.

`doc_changed` is one summary per grouped content edit, drained once per event
loop tick. `version` is the committed Loro version vector (which includes mark
operations); `source_digest` covers text only while `content_digest` also
covers marks, so a marks-only edit keeps the source digest but changes the
content digest and still publishes a change. `origin` names the edit source;
`document` is the open document's instance, so a change from a replaced
document is never applied to a new one. Caret moves, page/pan changes, status
updates and presence are read-only projections: they publish **no**
`doc_changed` and never advance `version`.

`state.local_state` (`loading`/`available`/`missing`/`failed`) and
`state.share_state` (`not_shared`/`catching_up`/`current`/`offline`/`refused`)
are reported separately: a missing local recovery copy does not prove a shared
document is empty, and a share with no recent acknowledgement is `offline`.
`*_state_detail` carries the failure/refusal text. The three `last_*_ack`
objects are `{document, content_generation, version}` and name the generation
each confirmation covered; a late completion for an older generation never
marks a newer revision saved. A peer ack confirms applied operations only — it
is **not** a claim about the peer's disk. `seed_granted` is true only for a
genuine new local document with confirmed creation, or a shared document with
an explicit create decision at its owning host; a recovered or late-arriving
note is never seeded over.

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
  "inline": false,
  "caption": "Terminal demo",
  "rect": {"x": 300, "y": 50, "w": 640, "h": 400},
  "page": 0,
  "window_id": 7,
  "title": "foot"
}
```

`window_id` and `title` are `null` for a dormant figure. `rect` is
zero-sized for a figure that isn't on the visible page.

`state.peers` is the live collaborator snapshot, self excluded, and is empty
when nobody is present:

```json
{
  "peer": "emthin-123-2",
  "name": "Ada",
  "cursor": 42,
  "connected": true,
  "last_seen_age_ms": 120
}
```

`cursor` is a byte offset clamped to this document's text. `connected` says
whether the reliable delta stream is joined. Presence expires after the
heartbeat timeout and is pruned; it is never persisted or written to Loro
history. The `collab_presence` notification carries the same `peers` list so
a transport can relay the optional `blob` field to the other peer.

`state.status` is the same compact line the bottom overlay draws, or `null`
when the document is quiet. `save_state` (`clean`/`unsaved`/`failed`),
`recovery_state` (`clean`/`recovered`/`failed`) and `preview_state`
(`current`/`updating`/`failed`) are separate: a recovery failure does not make
the explicit file look unsaved, and a failed preview leaves the file state
alone. `state.layout_error`, `state.save_error` and `state.recovery_error`
carry the full failure text; they remain until a successful layout or save
clears them. `state.save_warning` reports a non-fatal recovery such as loading
the verified backup pair.

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