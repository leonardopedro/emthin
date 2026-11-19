package com.emthin.pad;

/**
 * A document-bound stable Loro cursor plus its capture-time fallback.
 *
 * <p>The encoded position is opaque. It is valid only in the document whose
 * identity was recorded with it; another New/Open document must discard it
 * rather than reinterpret its bytes. Resolution snaps to a grapheme boundary
 * in the native bridge.
 */
final class StableAnchor {
    final String documentId;
    final String encoded;
    final int side;
    final int fallback;

    StableAnchor(String documentId, String encoded, int side, int fallback) {
        this.documentId = documentId == null ? "" : documentId;
        this.encoded = encoded == null ? "" : encoded;
        this.side = side < 0 ? -1 : (side > 0 ? 1 : 0);
        this.fallback = Math.max(0, fallback);
    }

    boolean belongsTo(String currentDocumentId) {
        return !encoded.isEmpty() && documentId.equals(currentDocumentId);
    }

    int resolve(long handle, String currentDocumentId) {
        if (handle == 0 || !belongsTo(currentDocumentId)) {
            return -1;
        }
        return DocBridge.resolveAnchor(handle, encoded, side, fallback);
    }
}
