package com.emthin.pad;

import java.util.HashMap;
import java.util.Map;
import java.util.UUID;

/**
 * Identity and request ownership for one Activity incarnation.
 *
 * <p>UI-thread code allocates captures before starting asynchronous work.
 * Workers never touch the live {@code DocModel}; they receive immutable
 * inputs and return their capture. A result is accepted only while the
 * document, Activity incarnation, mutation sequence and (for latest-only
 * lanes) request id still match. This keeps a late compile, citation render or
 * file completion from changing a newer document or a disposed view.
 *
 * <p>Every successful text <em>or mark</em> change calls
 * {@link #recordMutation()}. A marks-only edit therefore invalidates content
 * work even when the text revision is unchanged. Movement and presentation
 * changes do not bump the mutation sequence; they invalidate only the request
 * lane that owns the affected work.
 *
 * <p>Not thread-safe by design: allocate and invalidate on the UI thread.
 * {@link Owner} checks are synchronized so a worker may reject its own result
 * immediately before publishing it.
 */
final class PadSession {
    static final String RENDER = "render";
    static final String INSPECTOR = "inspector";
    static final String CITATION = "citation";
    static final String OPEN = "open";
    static final String FILE = "file";

    /** Immutable identity of one accepted piece of work. */
    static final class Owner {
        final String kind;
        final String documentId;
        final String instanceId;
        final long mutationSeq;
        final String presentationKey;
        final long requestId;

        private Owner(String kind, String documentId, String instanceId,
                      long mutationSeq, String presentationKey, long requestId) {
            this.kind = kind;
            this.documentId = documentId;
            this.instanceId = instanceId;
            this.mutationSeq = mutationSeq;
            this.presentationKey = presentationKey;
            this.requestId = requestId;
        }

        @Override public String toString() {
            return kind + "#" + requestId + "@" + mutationSeq;
        }
    }

    private String documentId = UUID.randomUUID().toString();
    private String instanceId = UUID.randomUUID().toString();
    private long mutationSeq;
    private long requestSeq;
    private boolean disposed;
    private final Map<String, Long> latestRequest = new HashMap<>();

    /** Allocate a new document identity for New or a successful Open. */
    synchronized void beginDocument() {
        documentId = UUID.randomUUID().toString();
        // New/Open is a new model incarnation too. Rotating both identities
        // makes an old document-bound result invalid even if UUIDs collide.
        instanceId = UUID.randomUUID().toString();
        mutationSeq++;
        latestRequest.clear();
    }

    /** Adopt a durable recovery identity after a cold launch or rotation. */
    synchronized void adoptDocument(String recoveredDocumentId) {
        if (recoveredDocumentId != null && !recoveredDocumentId.isEmpty()) {
            documentId = recoveredDocumentId;
        }
        instanceId = UUID.randomUUID().toString();
        mutationSeq++;
        latestRequest.clear();
    }

    /** Record one successful document mutation, including marks-only edits. */
    synchronized void recordMutation() {
        mutationSeq++;
    }

    /** Allocate the newest request in {@code kind}. */
    synchronized Owner capture(String kind, String presentationKey) {
        long requestId = ++requestSeq;
        latestRequest.put(kind, requestId);
        return new Owner(kind, documentId, instanceId, mutationSeq,
                presentationKey == null ? "" : presentationKey, requestId);
    }

    /**
     * Supersede all outstanding requests in {@code kind} without changing the
     * document. Used when a debounce or presentation change means an existing
     * result must not be published, before its replacement is captured.
     */
    synchronized void invalidate(String kind) {
        latestRequest.put(kind, ++requestSeq);
    }

    /** True when the result still belongs to this document incarnation/state. */
    synchronized boolean isCurrent(Owner owner) {
        return !disposed
                && owner != null
                && owner.documentId.equals(documentId)
                && owner.instanceId.equals(instanceId)
                && owner.mutationSeq == mutationSeq;
    }

    /** True when the result is current and is the newest request in its lane. */
    synchronized boolean isLatest(Owner owner) {
        if (!isCurrent(owner)) {
            return false;
        }
        Long newest = latestRequest.get(owner.kind);
        return newest != null && newest == owner.requestId;
    }

    /** True for file metadata that may be recorded for an older capture. */
    synchronized boolean sameDocument(Owner owner) {
        return !disposed
                && owner != null
                && owner.documentId.equals(documentId)
                && owner.instanceId.equals(instanceId);
    }

    synchronized String documentId() {
        return documentId;
    }

    /** Reject every outstanding result and every future result. */
    synchronized void dispose() {
        disposed = true;
        latestRequest.clear();
        requestSeq++;
    }

}
