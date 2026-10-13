# emthin IPC error contract

Every failed command answers with the `error` notification:

```json
{"jsonrpc":"2.0","method":"error","params":{"code":"unknown_figure","message":"unknown figure f9","category":"not_found","retryable":false,"remediation":"No figure with that key exists — check `list_state`'s `figures` and the figure keys in the document."}}
```

The contract is **tagged** (shape adapted from REA's error contract): a
client branches on `code`, never on `message`. The message is human
detail — its wording may change at any time and is the one field clients
must not parse.

The wire carries `code`, `message`, and (since 2026-10-13, the additive
extension this doc previously reserved) **`category`**, **`retryable`**
and **`remediation`** — a client can group failures, decide whether a
retry is worth attempting, and surface recovery text without fetching
this document. A client that parses only `code`/`message` is unaffected:
the fields were added, not changed. The unit test
`the_documented_error_semantics_match_the_wire_fields` pins the three
new fields to the table below cell-for-cell (byte-equal remediation), so
this table and the wire cannot drift.

## Closed code set

`code` values are exactly the variants of `IpcErrorCode`
(`crates/emthin/src/ipc/messages.rs`). The unit test
`the_documented_error_codes_are_the_only_codes` pins this table to that
enum in **both** directions: a code without a row here, or a row here
without a code, fails the suite. Adding a code is therefore a two-file
change — enum variant and doc row — and that is the point.

| code | category | retry | remediation | raised by |
|---|---|---|---|---|
| `not_bound` | not_found | no | The figure exists but no app is mapped into it. Spawn into it (or wait for `figure_bound`) before `close`/`focus`. | `close`, `focus` |
| `app_gone` | unavailable | no | The figure's app exited after binding. Re-`spawn`; the next `figure_bound` re-establishes the mapping. | `focus` |
| `unknown_figure` | not_found | no | No figure with that key exists — check `list_state`'s `figures` and the figure keys in the document. | `set_figure_size`, `clone_figure` |
| `voice_failed` | execution_failure | yes | The turn could not be queued (no `\voice` block under the target, a pending harness error, …). The message carries the reason; fix the block and retry. | `voice` (and async turn failures spliced into the transcript) |
| `io_failed` | io | no | The document path could not be read/written — check the path exists and is writable by the compositor's user. | `open_doc`, `save_doc` |

Categories are a closed set too: `not_found`, `unavailable`,
`execution_failure`, `io` (plus `malformed_input` for the inbound case
below). Retry is a boolean policy per code; it is never inferred from
the message.

## Inbound frames (client → compositor)

A frame that fails `parse_incoming` — invalid JSON, a missing or wrong
`jsonrpc` version, a missing `method`, or an unknown method — is
**logged and dropped, with no reply**. That is the current contract, and
it is recorded here because an undocumented silence is worse than the
silence itself: the category such a failure belongs to is
`malformed_input`, retry is no, and the remediation is "fix the client
frame" (`docs/ipc.md` is the frame catalog; `scripts/check-ipc-docsync`
keeps it honest). Emitting JSON-RPC `error` *responses* for these is a
possible future change; it is not what the code does today.

## What this contract is not

- It is not the JSON-RPC `error` response object (`{"error":{"code":…}}`
  in reply to a request). emthin's protocol is notification-shaped in
  both directions; `error` is a notification like the rest.
- It is not a stability promise about `message` wording — see above.
