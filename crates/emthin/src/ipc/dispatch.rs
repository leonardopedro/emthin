//! Control-message handlers.
//!
//! Every handler is a thin shim: validate, then call a `docui` method.
//! The document owns all policy — there is no layout arithmetic in this
//! file, which is exactly the inversion the rewrite was for.

use crate::ipc::{IncomingMessage, IpcErrorCode, IpcRect, OutgoingMessage, StateFigure};
use crate::EmthinState;

pub fn handle_ipc_message(state: &mut EmthinState, msg: IncomingMessage) {
    match msg {
        IncomingMessage::Spawn { cmd, args } => ipc_spawn(state, &cmd, &args),
        IncomingMessage::Close { figure } => ipc_close(state, &figure),
        IncomingMessage::Focus { figure } => ipc_focus(state, figure.as_deref()),
        IncomingMessage::SetFigureSize { figure, w, h } => {
            ipc_set_figure_size(state, &figure, w, h)
        }
        IncomingMessage::CloneFigure { figure } => ipc_clone_figure(state, &figure),
        IncomingMessage::GotoPage { page } => {
            tracing::debug!("IPC goto_page {page}");
            state.goto_page(page);
        }
        IncomingMessage::OpenDoc { path } => ipc_open_doc(state, &path),
        IncomingMessage::SaveDoc {
            path,
            format,
            force,
        } => ipc_save_doc(state, &path, format, force),
        IncomingMessage::ExportAscii { path } => ipc_export_ascii(state, &path),
        IncomingMessage::ImportAscii { path } => ipc_import_ascii(state, &path),
        IncomingMessage::SetTextSize { pt } => ipc_set_text_size(state, pt),
        IncomingMessage::InjectTouch { phase, slot, x, y } => state.inject_touch(phase, slot, x, y),
        IncomingMessage::ListState => ipc_list_state(state),
        IncomingMessage::Voice { text, block } => ipc_voice(state, block, text.as_deref()),
        IncomingMessage::DbusRouterAddRule { rule } => {
            tracing::debug!("IPC dbus_router_add_rule id={}", rule.id);
            state
                .dbus
                .send_rpc(emthin_dbus::router::BridgeCommand::AddRule(rule));
        }
        IncomingMessage::DbusRouterRemoveRule { id } => {
            tracing::debug!("IPC dbus_router_remove_rule id={id}");
            state
                .dbus
                .send_rpc(emthin_dbus::router::BridgeCommand::RemoveRule(id));
        }
        IncomingMessage::DbusRouterListRules => {
            tracing::debug!("IPC dbus_router_list_rules");
            state
                .dbus
                .send_rpc(emthin_dbus::router::BridgeCommand::ListRules);
        }
        IncomingMessage::CollabVersion => ipc_collab_version(state),
        IncomingMessage::CollabJoin {
            peer,
            version,
            scope,
        } => ipc_collab_join(state, &peer, &version, scope),
        IncomingMessage::CollabDelta {
            peer,
            seq,
            delta,
            scope,
            epoch,
        } => ipc_collab_delta(state, &peer, seq, &delta, scope, epoch),
        IncomingMessage::CollabAck { peer, seq, epoch } => ipc_collab_ack(state, &peer, seq, epoch),
        IncomingMessage::CollabLeave { peer } => {
            tracing::debug!("IPC collab_leave {peer}");
            state.collab.leave(&peer);
            state.needs_redraw = true;
        }
        IncomingMessage::CollabPresence { peer, blob, scope } => {
            ipc_collab_presence(state, &peer, &blob, scope)
        }
        IncomingMessage::CollabName { name } => {
            tracing::debug!("IPC collab_name {name}");
            state.collab_set_name(name);
            state.needs_redraw = true;
        }
        IncomingMessage::CollabShares { peer } => ipc_collab_shares(state, &peer),
        IncomingMessage::CollabGrant { peer, grant } => ipc_collab_grant(state, &peer, &grant),
        IncomingMessage::CollabRevoke { peer } => ipc_collab_revoke(state, &peer),
        IncomingMessage::CollabAnchor { peer, blob, epoch } => {
            ipc_collab_anchor(state, &peer, &blob, epoch)
        }
        IncomingMessage::HistoryList => ipc_history_list(state),
        IncomingMessage::HistoryPreview { frontier } => ipc_history_preview(state, &frontier),
        IncomingMessage::HistoryExport { frontier } => ipc_history_export(state, &frontier),
        IncomingMessage::InspectStart => {
            // When a primary session owns a state dir, observation also writes
            // an NDJSON trace beside it; a nested session still observes in
            // memory only. A sink failure leaves the in-memory log usable.
            match state.doc.inspect_sink_path() {
                Some(path) => {
                    if let Err(error) = state.inspect.enable_with(path) {
                        tracing::warn!("inspect sink unavailable: {error}");
                        state.inspect.enable();
                    }
                }
                None => state.inspect.enable(),
            }
        }
        IncomingMessage::InspectStop => {
            state.inspect.disable();
        }
        IncomingMessage::InspectState => ipc_inspect_state(state),
        IncomingMessage::SemanticOutline => ipc_semantic_outline(state),
        IncomingMessage::GotoDefinition { byte } => ipc_goto_definition(state, byte),
        IncomingMessage::RenameDefinition {
            def_index,
            new_name,
        } => ipc_rename_definition(state, def_index, &new_name),
        IncomingMessage::A11yNodes => ipc_a11y_nodes(state),
        IncomingMessage::LauncherEntries => ipc_launcher_entries(state),
        IncomingMessage::LauncherLaunch { figure } => ipc_launcher_launch(state, &figure),
        IncomingMessage::KernelRun {
            block_end,
            code,
            lang,
        } => ipc_kernel_run(state, block_end, &code, lang.as_deref()),
        IncomingMessage::KernelOutputs => ipc_kernel_outputs(state),
        IncomingMessage::CollabPublishDescriptor {
            owner,
            endpoint,
            rkey,
        } => ipc_collab_publish_descriptor(state, &owner, &endpoint, &rkey),
        IncomingMessage::CollabDescriptor { rkey } => ipc_collab_descriptor(state, &rkey),
        IncomingMessage::LintRun => ipc_lint_run(state),
        IncomingMessage::PluginCatalog => ipc_plugin_catalog(state),
        IncomingMessage::ClientIdentity => ipc_client_identity(state),
        IncomingMessage::ProofSearch { query, limit } => ipc_proof_search(state, &query, limit),
        IncomingMessage::ProofQuery => ipc_proof_query(state),
        IncomingMessage::ProofResult {
            share_id,
            document_key,
            proof_id,
            verdict,
        } => ipc_proof_result(state, &share_id, &document_key, &proof_id, &verdict),
    }
}

fn ipc_semantic_outline(state: &mut EmthinState) {
    let view = state.doc.semantic_view();
    let definitions = view
        .definitions
        .iter()
        .map(|row| {
            (
                row.index,
                row.name.clone(),
                row.uses,
                row.span.start,
                row.span.end,
            )
        })
        .collect();
    let uses = view
        .uses
        .iter()
        .map(|row| {
            (
                row.name.clone(),
                row.range.start,
                row.range.end,
                row.definition,
            )
        })
        .collect();
    let unresolved = view
        .unresolved
        .iter()
        .map(|row| (row.name.clone(), row.range.start, row.range.end))
        .collect();
    state.ipc.send(OutgoingMessage::SemanticOutline {
        definitions,
        uses,
        unresolved,
    });
}

fn ipc_goto_definition(state: &mut EmthinState, byte: usize) {
    let len = state.doc.model().len();
    let view = state.doc.semantic_view();
    let definition = match view.definition_at(byte).or_else(|| {
        view.use_at(byte)
            .and_then(|row| (row.definition != usize::MAX).then_some(row.definition))
    }) {
        Some(index) => index,
        None => return error(state, IpcErrorCode::NotFound, "no definition at byte"),
    };
    let target = view
        .definitions
        .get(definition)
        .map(|row| {
            row.name_range
                .clone()
                .unwrap_or_else(|| row.span.clone())
                .start
        })
        .unwrap_or(0);
    state.doc.model_mut().set_caret(target.min(len));
    state.needs_redraw = true;
}

fn ipc_rename_definition(state: &mut EmthinState, def_index: usize, new_name: &str) {
    let replacements = match state.doc.rename_definition(def_index, new_name) {
        Some(count) => count,
        None => {
            return error(
                state,
                IpcErrorCode::CollabFailed,
                "rename plan was empty or invalid",
            )
        }
    };
    state.ipc.send(OutgoingMessage::RenameResult {
        replacements,
        applied: true,
    });
    state.needs_redraw = true;
}

fn ipc_client_identity(state: &mut EmthinState) {
    let cred = state.ipc.client_credentials();
    state.ipc.send(OutgoingMessage::ClientIdentity {
        pid: cred.map(|c| c.pid),
        uid: cred.map(|c| c.uid),
        gid: cred.map(|c| c.gid),
    });
}

fn ipc_lint_run(state: &mut EmthinState) {
    let (generation, accepted) = match state.doc.lint_document() {
        Ok(generation) => (generation, true),
        Err(reason) => {
            tracing::info!("lint run skipped: {reason}");
            (0, false)
        }
    };
    state.ipc.send(OutgoingMessage::LintResult {
        generation,
        accepted,
    });
}

fn ipc_plugin_catalog(state: &mut EmthinState) {
    let plugins = vec![
        (
            "formal".to_owned(),
            state.doc.formals().is_available(),
            state.doc.formals().calls(),
        ),
        // The voice bridge tracks one in-flight turn, not a lifetime count.
        ("voice".to_owned(), state.doc.voice().is_available(), 0),
        (
            "diagnostics".to_owned(),
            state.doc.diagnostics().is_available(),
            state.doc.diagnostics().runs(),
        ),
    ];
    state.ipc.send(OutgoingMessage::PluginCatalog { plugins });
}

fn ipc_collab_publish_descriptor(state: &mut EmthinState, owner: &str, endpoint: &str, rkey: &str) {
    use emthin_atproto::{DocumentIdentity, SessionDescriptor};

    let revision = state.doc.model().content_generation();
    let document_id = format!(
        "emthin:sha256:{:016x}",
        revision ^ (state.doc.model().text().len() as u64)
    );
    let descriptor = match SessionDescriptor::builder()
        .document(DocumentIdentity {
            id: document_id,
            kind: "mathed".into(),
            title: "emthin document".into(),
        })
        .owner(owner)
        .endpoint(endpoint)
        .revision(revision)
        // Coarse expiry: the live endpoint's lease still governs admission.
        .validity("1970-01-01T00:00:00Z", "9999-12-31T23:59:59Z")
        .build()
    {
        Ok(descriptor) => descriptor,
        Err(reason) => {
            return error(
                state,
                IpcErrorCode::CollabFailed,
                format!("invalid descriptor: {reason}"),
            )
        }
    };
    match emthin_atproto::publish(&mut state.atproto_pds, rkey, &descriptor) {
        Ok(commit) => state.ipc.send(OutgoingMessage::CollabDescriptorPublished {
            rkey: commit.rkey,
            revision: commit.revision,
        }),
        Err(reason) => {
            // Publication is best-effort: never on the save or edit path.
            tracing::info!("descriptor publication skipped: {reason}");
            error(state, IpcErrorCode::CollabFailed, reason.to_string());
        }
    }
}

fn ipc_collab_descriptor(state: &mut EmthinState, rkey: &str) {
    match emthin_atproto::fetch(&mut state.atproto_pds, rkey) {
        Ok(descriptor) => state.ipc.send(OutgoingMessage::CollabDescriptor {
            json: descriptor.map(|d| d.to_json()),
        }),
        Err(reason) => error(state, IpcErrorCode::CollabFailed, reason.to_string()),
    }
}

fn ipc_kernel_run(state: &mut EmthinState, block_end: usize, code: &str, lang: Option<&str>) {
    let accepted = state.doc.kernel_run(block_end, code, lang);
    let (generation, ok) = match accepted {
        Ok(generation) => (generation, true),
        Err(reason) => {
            tracing::info!("kernel_run rejected: {reason}");
            (0, false)
        }
    };
    state.ipc.send(OutgoingMessage::KernelSubmitted {
        generation,
        accepted: ok,
    });
}

fn ipc_kernel_outputs(state: &mut EmthinState) {
    let regions = state
        .doc
        .kernel()
        .outputs()
        .iter()
        .map(|(end, markup)| (*end, markup.clone()))
        .collect();
    state.ipc.send(OutgoingMessage::KernelOutputs { regions });
}

fn ipc_launcher_entries(state: &mut EmthinState) {
    let entries = crate::docui::launcher::entries(state.doc.figures())
        .into_iter()
        .map(|entry| (entry.key, entry.label, entry.page))
        .collect();
    state.ipc.send(OutgoingMessage::LauncherList { entries });
}

fn ipc_launcher_launch(state: &mut EmthinState, figure: &str) {
    let spawned = state.relaunch_dormant(figure);
    state.ipc.send(OutgoingMessage::LauncherLaunched {
        figure: figure.to_owned(),
        spawned,
    });
}

fn ipc_a11y_nodes(state: &mut EmthinState) {
    let model = state.doc.model();
    let index = crate::docui::a11y::A11yIndex::build(model.text(), model.scan(), model.segments());
    let nodes = index
        .nodes
        .into_iter()
        .map(|node| {
            (
                node.role.to_string(),
                node.label,
                node.value,
                node.range.as_ref().map(|range| range.start),
                node.range.as_ref().map(|range| range.end),
            )
        })
        .collect();
    state.ipc.send(OutgoingMessage::A11yNodes { nodes });
}

fn ipc_proof_search(state: &mut EmthinState, query: &str, limit: usize) {
    // A search before any query builds the view first, so the second query is
    // independent of whether `proof_query` was called. `proof_view_cached`
    // keeps an already-accepted verdict instead of silently rebuilding it.
    let key = state.doc.document_key().unwrap_or("untitled").to_string();
    let _ = state.doc.proof_view_cached(state.collab.share_id(), &key);
    let matches = state.doc.proof_search(query, limit);
    state.ipc.send(OutgoingMessage::ProofMatches {
        definitions: matches
            .definitions
            .into_iter()
            .map(|row| (row.name, row.start, row.end))
            .collect(),
        references: matches
            .references
            .into_iter()
            .map(|row| (row.name, row.start, row.end))
            .collect(),
        missing: matches.missing,
    });
}

fn ipc_proof_query(state: &mut EmthinState) {
    let share = crate::ipc::hex::encode(state.collab.share_id());
    let key = state.doc.document_key().unwrap_or("untitled").to_string();
    let view = state.doc.proof_view_cached(state.collab.share_id(), &key);
    let proof_id = view.proof_id().map(str::to_string);
    let definitions = view
        .definitions
        .into_iter()
        .map(|row| (row.name, row.start, row.end, row.uses))
        .collect();
    let references = view
        .references
        .into_iter()
        .map(|row| (row.name, row.start, row.end, row.target))
        .collect();
    state.ipc.send(OutgoingMessage::ProofView {
        share_id: share,
        document_key: key,
        revision: view.revision,
        definitions,
        references,
        missing: view.missing,
        proof_id,
        verdict: view.proof.map(|p| p.verdict),
    });
}

fn ipc_proof_result(
    state: &mut EmthinState,
    share_id: &str,
    document_key: &str,
    proof_id: &str,
    verdict: &str,
) {
    use crate::docui::proof_index::{ProofResult, ViewOwner};
    let result = ProofResult {
        owner: ViewOwner::new(share_id, document_key),
        proof_id: proof_id.to_string(),
        verdict: verdict.to_string(),
    };
    // A late result for a different proof/document is ignored; no document
    // state, save baseline or undo entry changes.
    state.doc.apply_proof_result(result);
}

fn ipc_inspect_state(state: &mut EmthinState) {
    let settled = state.inspect_state();
    state.ipc.send(OutgoingMessage::InspectState {
        enabled: state.inspect.is_on(),
        local: settled.local,
        share: settled.share,
        frontier: settled.frontier,
        accepted: settled.accepted,
        peers: settled.peers,
        reason: settled.reason,
    });
}

/// Emit an `error` notification: a closed-set `code` for clients to
/// branch on, the human `message` for the transcript and logs. The code's
/// category/retry/remediation semantics live in `docs/error-contract.md`.
fn error(state: &mut EmthinState, code: IpcErrorCode, message: impl Into<String>) {
    let message = message.into();
    tracing::warn!("IPC error [{code:?}]: {message}");
    state.ipc.send(OutgoingMessage::Error { code, message });
}

fn ipc_collab_version(state: &mut EmthinState) {
    let version = crate::ipc::hex::encode(&state.doc.model().math_doc().version_vector());
    tracing::debug!("IPC collab_version ({} hex chars)", version.len());
    state.ipc.send(OutgoingMessage::CollabVersion { version });
}

fn ipc_collab_join(state: &mut EmthinState, peer: &str, version: &str, scope: Option<u64>) {
    let version = match crate::ipc::hex::decode(version) {
        Ok(v) => v,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e),
    };
    if state.collab.has_explicit_grants() {
        let principal = emthin_collab::grants::Principal::verified(peer.to_string());
        if !state
            .collab
            .permitted(&principal, emthin_collab::grants::Grant::Read)
        {
            return error(
                state,
                IpcErrorCode::CollabFailed,
                "no read grant for this share",
            );
        }
    }
    let result = if state.collab.has_peer(peer) {
        state
            .collab
            .reconnect_scoped(state.doc.model().math_doc(), peer, &version, scope)
    } else {
        state
            .collab
            .join_scoped(state.doc.model().math_doc(), peer, &version, scope)
    };
    let out = match result {
        Ok(out) => out,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e.to_string()),
    };
    tracing::debug!("IPC collab_join {peer}");
    // Arm the share readiness clock on the handshake. Without this the share
    // stays `not_shared`, `note_peer_ack` returns early and the offline timeout
    // in tick.rs can never fire — a peer that vanishes is never reported.
    state.doc.note_share_joined();
    state.inspect_event(crate::docui::inspect::EventKind::Handshake, peer, "", 0, "");
    state.ipc.send(OutgoingMessage::CollabDelta {
        peer: out.peer,
        seq: out.seq,
        delta: crate::ipc::hex::encode(&out.delta),
        scope: out.scope,
        // The handshake assigned this peer's epoch; later frames must echo it
        // so a reconnect (which assigns a fresh one) fences the old stream.
        epoch: state.collab.peer_epoch(peer),
    });
    state.collab_publish_presence(true);
}

fn ipc_collab_presence(state: &mut EmthinState, peer: &str, blob: &str, scope: Option<u64>) {
    let blob = match crate::ipc::hex::decode(blob) {
        Ok(blob) => blob,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e),
    };
    if state.collab.has_explicit_grants() {
        let principal = emthin_collab::grants::Principal::verified(peer.to_string());
        if !state
            .collab
            .permitted(&principal, emthin_collab::grants::Grant::Read)
        {
            return error(
                state,
                IpcErrorCode::CollabFailed,
                "no read grant for this share",
            );
        }
    }
    if let Err(e) = state.collab_apply_presence_scoped(&blob, scope) {
        return error(
            state,
            IpcErrorCode::CollabFailed,
            format!("presence from {peer}: {e}"),
        );
    }
    tracing::debug!("IPC collab_presence {peer} ({} bytes)", blob.len());
}

fn ipc_collab_delta(
    state: &mut EmthinState,
    peer: &str,
    seq: u64,
    delta: &str,
    scope: Option<u64>,
    epoch: Option<u64>,
) {
    let delta = match crate::ipc::hex::decode(delta) {
        Ok(delta) => delta,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e),
    };
    // A share with explicit grants enforces them through the session, which
    // also refuses an epoch a revoke invalidated; an unconfigured share keeps
    // the documented unscoped compatibility path. Either way the frame must
    // match the live connection epoch and document scope.
    let outcome = {
        let model = state.doc.model_mut();
        model.set_change_origin(crate::docui::change::ChangeOrigin::Peer);
        let doc = model.math_doc_mut();
        if state.collab.has_explicit_grants() {
            let principal = emthin_collab::grants::Principal::verified(peer.to_string());
            state
                .collab
                .receive_granted(doc, &principal, peer, seq, &delta, scope, epoch)
        } else {
            state
                .collab
                .receive_v2(doc, peer, seq, &delta, scope, epoch)
        }
    };
    // `math_doc_mut` dirtied the model; rebuild the scan even when the
    // import failed after applying some of the buffered commands.
    state.doc.model_mut().rescan();
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(emthin_collab::SessionError::Denied) => {
            state.inspect_event(
                crate::docui::inspect::EventKind::Refused,
                peer,
                "",
                0,
                "no edit grant for this share",
            );
            return error(
                state,
                IpcErrorCode::CollabFailed,
                "no edit grant for this share",
            );
        }
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e.to_string()),
    };
    let outbound = match outcome {
        emthin_collab::ReceiveOutcome::Applied {
            operations,
            outbound,
        } => {
            state.inspect_event(
                crate::docui::inspect::EventKind::AcceptedChanges,
                peer,
                "",
                operations as u64,
                "",
            );
            outbound
        }
        emthin_collab::ReceiveOutcome::Duplicate => {
            state.doc.note_late_arrival();
            return;
        }
        emthin_collab::ReceiveOutcome::Stale => {
            tracing::debug!("ignored collab delta from a replaced document scope");
            state.doc.note_late_arrival();
            return;
        }
        emthin_collab::ReceiveOutcome::Gap { expected } => {
            tracing::warn!(
                "collab delta from {peer} arrived at seq {seq} with a gap; expected {expected}"
            );
            state.doc.note_late_arrival();
            return;
        }
    };
    tracing::debug!(
        "IPC collab_delta {peer} seq={seq} ({} forwarded)",
        outbound.len()
    );
    state.collab_after_remote();
    for out in outbound {
        let epoch = state.collab.peer_epoch(&out.peer);
        state.ipc.send(OutgoingMessage::CollabDelta {
            peer: out.peer,
            seq: out.seq,
            delta: crate::ipc::hex::encode(&out.delta),
            scope: out.scope,
            epoch,
        });
    }
}

fn ipc_collab_ack(state: &mut EmthinState, peer: &str, seq: u64, epoch: Option<u64>) {
    // An acknowledgement from a replaced connection must not advance the
    // stream, or a revoked peer could release frames it should never see.
    if let Err(e) = state.collab.ack_v2(peer, seq, epoch) {
        return error(state, IpcErrorCode::CollabFailed, e.to_string());
    }
    // The cumulative ack confirms operations through `seq`; it says nothing
    // about the peer's disk, so it is recorded as a non-durable acknowledgement
    // of the generation current at this end (K2).
    state.doc.note_peer_ack(true);
    tracing::debug!("IPC collab_ack {peer} through {seq}");
}

fn ipc_collab_shares(state: &mut EmthinState, peer: &str) {
    // Denied and unknown identities both see the same outward result.
    let visible = state.collab_share_visible(peer);
    state.ipc.send(OutgoingMessage::CollabShareAccess {
        peer: peer.to_string(),
        visible,
    });
}

fn ipc_collab_grant(state: &mut EmthinState, peer: &str, grant: &str) {
    if let Err(message) = state.collab_grant(peer, grant) {
        error(state, IpcErrorCode::CollabFailed, message);
    }
    state.needs_redraw = true;
}

fn ipc_collab_revoke(state: &mut EmthinState, peer: &str) {
    if let Err(message) = state.collab_revoke(peer) {
        error(state, IpcErrorCode::CollabFailed, message);
    }
    state.needs_redraw = true;
}

fn ipc_collab_anchor(state: &mut EmthinState, peer: &str, blob: &str, epoch: Option<u64>) {
    let blob = match crate::ipc::hex::decode(blob) {
        Ok(blob) => blob,
        Err(e) => return error(state, IpcErrorCode::CollabFailed, e),
    };
    if let Err(message) = state.collab_apply_anchor(peer, &blob, epoch) {
        error(state, IpcErrorCode::CollabFailed, message);
    }
}

fn ipc_history_list(state: &mut EmthinState) {
    state.ipc.send(OutgoingMessage::History {
        frontiers: state.doc.history_frontiers(),
    });
}

fn ipc_history_preview(state: &mut EmthinState, frontier: &str) {
    use crate::docui::history::HistoryStatus;
    let share_id = *state.collab.share_id();
    let message = match state.doc.history_preview(frontier, share_id) {
        Ok(preview) => OutgoingMessage::HistoryPreview {
            frontier: frontier.to_string(),
            share_id: crate::ipc::hex::encode(&preview.share_id),
            owner: preview.owner,
            seq: preview.seq,
            text: preview.text,
            marks: preview
                .marks
                .into_iter()
                .map(|(range, key)| (range.start, range.end, key))
                .collect(),
            content_digest: preview.content_digest,
            status: "available",
            reason: None,
        },
        Err(HistoryStatus::Unavailable { reason }) => OutgoingMessage::HistoryPreview {
            frontier: frontier.to_string(),
            share_id: crate::ipc::hex::encode(&share_id),
            owner: String::new(),
            seq: 0,
            text: String::new(),
            marks: Vec::new(),
            content_digest: String::new(),
            status: "unavailable",
            reason: Some(reason),
        },
        Err(HistoryStatus::Available) => unreachable!("Available is not an error status"),
    };
    state.ipc.send(message);
}

fn ipc_history_export(state: &mut EmthinState, frontier: &str) {
    let share_id = *state.collab.share_id();
    if let Err(message) = state.doc.promote_history_copy(frontier, share_id) {
        return error(state, IpcErrorCode::CollabFailed, message);
    }
    // A copy export replaces the live document; discard old collab scope and
    // readiness just as an Open would.
    state.collab_reset_document_scope();
    state.needs_redraw = true;
}

fn ipc_spawn(state: &mut EmthinState, cmd: &str, args: &[String]) {
    tracing::debug!("IPC spawn {cmd} {args:?}");
    let display = state.xwayland.display();
    crate::util::spawn_child(cmd, args, display, state);
}

fn ipc_close(state: &mut EmthinState, figure: &str) {
    let Some(app_id) = state.doc.figures().get(figure).and_then(|f| f.app_id) else {
        return error(
            state,
            IpcErrorCode::NotBound,
            format!("no app bound to figure {figure}"),
        );
    };
    tracing::debug!("IPC close figure={figure} app={app_id}");
    if let Some(app) = state.apps.get(app_id) {
        if let Some(toplevel) = app.window.toplevel() {
            toplevel.send_close();
        }
    }
}

fn ipc_focus(state: &mut EmthinState, figure: Option<&str>) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let target = match figure {
        // `None` = the document itself, which owns no Wayland surface, so
        // the seat has nothing to focus. Clearing is what puts typing
        // back on the caret.
        None => None,
        Some(key) => {
            let Some(app_id) = state.doc.figures().get(key).and_then(|f| f.app_id) else {
                return error(
                    state,
                    IpcErrorCode::NotBound,
                    format!("no app bound to figure {key}"),
                );
            };
            match state.apps.get(app_id) {
                Some(app) => Some(crate::KeyboardFocusTarget::from(app.window.clone())),
                None => {
                    return error(
                        state,
                        IpcErrorCode::AppGone,
                        format!("figure {key}'s app is gone"),
                    )
                }
            }
        }
    };
    tracing::debug!("IPC focus figure={figure:?}");
    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
    keyboard.set_focus(state, target, serial);
    state.needs_redraw = true;
}

fn ipc_set_figure_size(state: &mut EmthinState, figure: &str, w: i32, h: i32) {
    let Some(fig) = state.doc.figures().get(figure).cloned() else {
        return error(
            state,
            IpcErrorCode::UnknownFigure,
            format!("unknown figure {figure}"),
        );
    };
    tracing::debug!("IPC set_figure_size figure={figure} {w}x{h}");
    if crate::docui::edit::set_figure_size(state.doc.model_mut(), &fig, w, h) {
        // Reflow reconfigures the bound app through the normal
        // relayout path.
        state.relayout_document();
        state.needs_redraw = true;
    }
}

fn ipc_clone_figure(state: &mut EmthinState, figure: &str) {
    let Some(fig) = state.doc.figures().get(figure).cloned() else {
        return error(
            state,
            IpcErrorCode::UnknownFigure,
            format!("unknown figure {figure}"),
        );
    };
    tracing::debug!("IPC clone_figure figure={figure}");
    crate::docui::edit::clone_figure(state.doc.model_mut(), &fig);
    state.relayout_document();
    state.needs_redraw = true;
}

/// Queue a voice turn. The harness runs on a worker thread, so this
/// returns immediately: the outcome arrives as a `voice_result`
/// notification (or `error`) from the tick that polls the bridge.
fn ipc_voice(state: &mut EmthinState, block: Option<usize>, text: Option<&str>) {
    tracing::debug!("IPC voice block={block:?} text={text:?}");
    match state.doc.voice_submit(block, text) {
        Ok(submit) => tracing::info!(
            "IPC voice: turn queued for block #{} (append_utterance={}, say={})",
            submit.block_index,
            submit.append_utterance,
            submit.say
        ),
        Err(reason) => error(state, IpcErrorCode::VoiceFailed, format!("voice: {reason}")),
    }
}

fn ipc_open_doc(state: &mut EmthinState, path: &str) {
    let path = std::path::PathBuf::from(path);
    match state.doc.open_document(&path) {
        Ok(()) => {
            // The session's peers, lines and presence all belong to the
            // previous document; reset them as one scope.
            state.collab_reset_document_scope();
            state.relayout_document();
            tracing::info!("opened document {}", path.display());
        }
        Err(reason) => error(state, IpcErrorCode::IoFailed, reason.to_string()),
    }
}

fn ipc_save_doc(
    state: &mut EmthinState,
    path: &str,
    format: Option<crate::docui::file::SaveFormat>,
    force: bool,
) {
    let path = std::path::PathBuf::from(path);
    match state.doc.save_document(&path, format, force) {
        Ok(saved) => {
            tracing::info!(
                "saved document {} as {} ({})",
                saved.path.display(),
                saved.kind.as_str(),
                saved.digest
            );
            state.ipc.send(OutgoingMessage::DocSaved {
                path: saved.path.display().to_string(),
                format: saved.kind.as_str().to_string(),
                digest: saved.digest,
            });
        }
        Err(crate::docui::file::FileError::Conflict(target)) => error(
            state,
            IpcErrorCode::DocConflict,
            format!(
                "{} changed on disk since it was opened; repeat with force:true to replace it",
                target.display()
            ),
        ),
        Err(reason) => error(state, IpcErrorCode::IoFailed, reason.to_string()),
    }
}

fn ipc_export_ascii(state: &mut EmthinState, path: &str) {
    let path = std::path::PathBuf::from(path);
    match std::fs::write(&path, state.doc.export_ascii()) {
        Ok(()) => {
            tracing::info!("exported ASCII projection {}", path.display());
            state.ipc.send(OutgoingMessage::AsciiExported {
                path: path.display().to_string(),
            });
        }
        Err(e) => error(
            state,
            IpcErrorCode::IoFailed,
            format!("could not write {}: {e}", path.display()),
        ),
    }
}

fn ipc_import_ascii(state: &mut EmthinState, path: &str) {
    let path = std::path::PathBuf::from(path);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            return error(
                state,
                IpcErrorCode::IoFailed,
                format!("could not read {}: {e}", path.display()),
            );
        }
    };
    match state.doc.import_ascii(&text) {
        Ok(()) => {
            // A successful import replaces the document just like Open.
            state.collab_reset_document_scope();
            state.relayout_document();
            tracing::info!("imported ASCII projection {}", path.display());
        }
        Err(reason) => error(state, IpcErrorCode::NotAscii, reason),
    }
}

/// Set the reading size. Clamping is the document's policy; this shim
/// reports the size that actually took effect, even when it was unchanged.
fn ipc_set_text_size(state: &mut EmthinState, pt: f32) {
    let changed = state.doc.set_text_size(pt);
    if changed {
        state.relayout_document();
        state.needs_redraw = true;
    }
    let pt = state.doc.text_size();
    tracing::info!(
        "IPC set_text_size {pt}pt ({} pages, changed={changed})",
        state.doc.page_count()
    );
    state.ipc.send(OutgoingMessage::TextSizeChanged {
        pt,
        page_count: state.doc.page_count(),
    });
}

fn ipc_list_state(state: &mut EmthinState) {
    state.collab_prune_presence();
    let peers = state.collab_peers();
    let collab_local_peer = state.collab.local_peer().to_string();
    let collab_local_name = state.collab_local_name.clone();
    let mut status_model = state.doc.status(peers.len());
    // The inspect trace is owned by the compositor state, not the document.
    status_model.inspect_events = state.inspect.events().count();
    let status = Box::new(status_model.line());
    let save_state = status_model.save.as_str();
    let recovery_state = status_model.recovery.as_str();
    let preview_state = status_model.preview.as_str();
    let layout_error = Box::new(status_model.layout_error.clone());
    let save_error = Box::new(state.doc.file_save_error().map(str::to_string));
    let recovery_error = Box::new(state.doc.recovery_error().map(str::to_string));
    let save_warning = Box::new(state.doc.persistence_warning().map(str::to_string));
    let readiness = state.doc.readiness_snapshot();
    let ack_tuple = |generation: Option<crate::docui::readiness::Generation>| {
        generation.map(|generation| {
            (
                generation.document,
                generation.content_generation,
                generation.version,
            )
        })
    };
    let last_recovery_ack = ack_tuple(readiness.last_recovery_ack.clone());
    let last_explicit_ack = ack_tuple(readiness.last_explicit_ack.clone());
    let last_peer_ack = ack_tuple(readiness.last_peer_ack.clone());
    let doc = Box::new(state.doc.model().text().to_string());
    let page = state.doc.current_page();
    let page_count = state.doc.page_count();
    let text_size = state.doc.text_size();
    let view_zoom = state.doc.view_zoom();
    let pan = state.doc.view_pan();
    let citation_style = Box::new(
        state
            .doc
            .citation_popup()
            .style_override()
            .map(str::to_string),
    );
    let citation_locale = Box::new(
        state
            .doc
            .citation_popup()
            .locale_override()
            .map(str::to_string),
    );
    let figures = state
        .doc
        .figures()
        .figures()
        .iter()
        .map(|f| StateFigure {
            key: f.key.clone(),
            id: f.spec.id.clone(),
            caption: state.doc.model().text()[f.span.clone()].trim().to_string(),
            // `docs/ipc.md`: "rect is zero-sized for a figure that isn't on the
            // visible page". It was not: an off-page figure keeps the `(0, 0)`
            // placeholder `page_rect_for` hands it, so a frontend drawing
            // `state.figures` stacked every off-page figure in the output's
            // top-left corner.
            rect: if f.page == Some(page) {
                IpcRect {
                    x: f.rect.loc.x,
                    y: f.rect.loc.y,
                    w: f.rect.size.w,
                    h: f.rect.size.h,
                }
            } else {
                IpcRect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                }
            },
            page: f.page.unwrap_or(0),
            window_id: f.app_id,
            title: f.title.clone(),
        })
        .collect();
    state.ipc.send(OutgoingMessage::State {
        page,
        page_count,
        text_size,
        view_zoom,
        view_pan_x: pan.x,
        view_pan_y: pan.y,
        citation_style,
        citation_locale,
        doc,
        figures,
        peers,
        collab_local_peer,
        collab_local_name,
        status,
        save_state,
        recovery_state,
        preview_state,
        layout_error,
        save_error,
        recovery_error,
        save_warning,
        local_state: readiness.local,
        share_state: readiness.share,
        local_state_detail: Box::new(readiness.local_detail),
        share_state_detail: Box::new(readiness.share_detail),
        seed_granted: readiness.seeded,
        last_recovery_ack: Box::new(last_recovery_ack),
        last_explicit_ack: Box::new(last_explicit_ack),
        last_peer_ack: Box::new(last_peer_ack),
    });
}
