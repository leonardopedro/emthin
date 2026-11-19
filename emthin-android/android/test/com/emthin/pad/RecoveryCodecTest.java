package com.emthin.pad;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.Map;

/** Host tests for the A3 envelope; run with {@code test-java.sh}. */
public final class RecoveryCodecTest {
    private static int checks;

    public static void main(String[] args) {
        roundTripPreservesIdentityAndView();
        checksumAndTruncationRetainEvidence();
        lengthAndVersionMismatchesAreExplicit();
        badMagicAndOversizedMetadataAreRejected();
        quarantineNamesAreStableAndDistinct();
        System.out.println("recovery codec tests passed (" + checks + " checks)");
    }

    private static void roundTripPreservesIdentityAndView() {
        RecoveryCodec.Metadata metadata = new RecoveryCodec.Metadata();
        metadata.generation = 17;
        metadata.documentId = "doc-ü";
        metadata.logicalFingerprint = "a".repeat(64);
        metadata.displayName = "Lecture notes";
        metadata.uri = "content://example/doc";
        metadata.fileSnapshot = false;
        metadata.caret = 4;
        metadata.selectionEnd = 9;
        metadata.scrollY = 120;
        metadata.scrollWidth = 800;
        metadata.readingAnchorEncoded = "cursor-bytes";
        metadata.readingAnchorSide = -1;
        metadata.readingFallback = 4;
        metadata.showMarkers = true;
        metadata.inspectorOpen = true;
        metadata.query = "Loro";
        metadata.folds.add(Arrays.asList("a", "b"));
        metadata.citationChoices.add("apa");
        byte[] snapshot = "snapshot\nbytes".getBytes(StandardCharsets.UTF_8);

        byte[] envelope = RecoveryCodec.encode(metadata, snapshot);
        try {
            RecoveryCodec.Record record = RecoveryCodec.decode(envelope);
            check(record.metadata.generation == 17, "generation round-trips");
            check(record.metadata.documentId.equals("doc-ü"), "document identity round-trips");
            check(record.metadata.logicalFingerprint.equals("a".repeat(64)),
                    "logical fingerprint round-trips");
            check(record.metadata.uri.equals("content://example/doc"), "URI round-trips");
            check(!record.metadata.fileSnapshot, "file snapshot flag round-trips");
            check(record.metadata.caret == 4 && record.metadata.selectionEnd == 9,
                    "selection round-trips");
            check(record.metadata.scrollY == 120 && record.metadata.scrollWidth == 800,
                    "reading size/position round-trip");
            check(record.metadata.readingAnchorEncoded.equals("cursor-bytes")
                    && record.metadata.readingAnchorSide == -1
                    && record.metadata.readingFallback == 4,
                    "stable reading anchor round-trips");
            check(record.metadata.showMarkers && record.metadata.inspectorOpen,
                    "view flags round-trip");
            check(record.metadata.query.equals("Loro"), "accepted query round-trips");
            check(record.metadata.folds.size() == 1
                    && record.metadata.folds.get(0).equals(Arrays.asList("a", "b")),
                    "folds round-trip");
            check(record.metadata.citationChoices.equals(Arrays.asList("apa")),
                    "citation choices round-trip");
            check(Arrays.equals(record.snapshot, snapshot), "snapshot bytes round-trip");
        } catch (RecoveryCodec.DecodeException e) {
            throw new AssertionError("valid envelope rejected: " + e.getMessage());
        }
    }

    private static void checksumAndTruncationRetainEvidence() {
        byte[] envelope = basicEnvelope(1, "doc", "snapshot");
        byte[] corrupt = envelope.clone();
        corrupt[corrupt.length - 40] ^= 0x40;
        expectFailure(corrupt, "checksum");
        byte[] shortened = Arrays.copyOf(envelope, envelope.length - 5);
        expectFailure(shortened, "checksum");
        byte[] truncated = Arrays.copyOf(envelope, 10);
        expectFailure(truncated, "truncated");
    }

    private static void lengthAndVersionMismatchesAreExplicit() {
        Map<String, Object> metadata = basicMetadata(2, "doc", 8);
        metadata.put("snapshotLength", 9);
        byte[] mismatch = envelope(RecoveryCodec.Json.write(metadata), "snapshot".getBytes(StandardCharsets.UTF_8));
        expectFailure(mismatch, "snapshot length");

        Map<String, Object> future = basicMetadata(2, "doc", 8);
        future.put("version", 2);
        byte[] futureEnvelope = envelope(RecoveryCodec.Json.write(future), "snapshot".getBytes(StandardCharsets.UTF_8));
        try {
            RecoveryCodec.decode(futureEnvelope);
            throw new AssertionError("future version accepted");
        } catch (RecoveryCodec.DecodeException e) {
            check(e.futureVersion, "future version is reported explicitly");
            check(Arrays.equals(e.raw, futureEnvelope), "future-version bytes are retained");
        }
    }

    private static void badMagicAndOversizedMetadataAreRejected() {
        byte[] bad = basicEnvelope(1, "doc", "snapshot");
        bad[0] = 'X';
        expectFailure(bad, "magic");

        byte[] oversized = new byte[64];
        System.arraycopy(RecoveryCodec.MAGIC, 0, oversized, 0, RecoveryCodec.MAGIC.length);
        oversized[8] = (byte) 0xff;
        oversized[9] = (byte) 0xff;
        oversized[10] = (byte) 0xff;
        oversized[11] = 0x7f;
        expectFailure(oversized, "metadata length");
    }

    private static void quarantineNamesAreStableAndDistinct() {
        byte[] a = "rejected-a".getBytes(StandardCharsets.UTF_8);
        byte[] b = "rejected-b".getBytes(StandardCharsets.UTF_8);
        check(RecoveryCodec.checksumName(a).equals(RecoveryCodec.checksumName(a.clone())),
                "quarantine name is stable for identical bytes");
        check(!RecoveryCodec.checksumName(a).equals(RecoveryCodec.checksumName(b)),
                "distinct rejected bytes get distinct names");
        check(RecoveryCodec.checksumName(a).length() == 64, "quarantine name is SHA-256 hex");
    }

    private static byte[] basicEnvelope(long generation, String documentId, String text) {
        byte[] snapshot = text.getBytes(StandardCharsets.UTF_8);
        return envelope(RecoveryCodec.Json.write(basicMetadata(generation, documentId, snapshot.length)),
                snapshot);
    }

    private static Map<String, Object> basicMetadata(long generation, String documentId, int length) {
        Map<String, Object> metadata = new LinkedHashMap<>();
        metadata.put("version", 1);
        metadata.put("generation", generation);
        metadata.put("documentId", documentId);
        metadata.put("logicalFingerprint", "");
        metadata.put("snapshotLength", length);
        return metadata;
    }

    /** Build a correctly checksummed envelope for negative-path tests. */
    private static byte[] envelope(String json, byte[] snapshot) {
        byte[] metadata = json.getBytes(StandardCharsets.UTF_8);
        byte[] out = new byte[RecoveryCodec.MAGIC.length + 4 + metadata.length
                + snapshot.length + RecoveryCodec.CHECKSUM];
        System.arraycopy(RecoveryCodec.MAGIC, 0, out, 0, RecoveryCodec.MAGIC.length);
        out[8] = (byte) metadata.length;
        out[9] = (byte) (metadata.length >>> 8);
        out[10] = (byte) (metadata.length >>> 16);
        out[11] = (byte) (metadata.length >>> 24);
        System.arraycopy(metadata, 0, out, 12, metadata.length);
        System.arraycopy(snapshot, 0, out, 12 + metadata.length, snapshot.length);
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            digest.update(out, 0, out.length - RecoveryCodec.CHECKSUM);
            byte[] checksum = digest.digest();
            System.arraycopy(checksum, 0, out, out.length - RecoveryCodec.CHECKSUM, checksum.length);
        } catch (Exception e) {
            throw new AssertionError(e);
        }
        return out;
    }

    private static void expectFailure(byte[] envelope, String reasonPart) {
        try {
            RecoveryCodec.decode(envelope);
            throw new AssertionError("invalid envelope accepted: " + reasonPart);
        } catch (RecoveryCodec.DecodeException e) {
            check(e.getMessage().toLowerCase().contains(reasonPart),
                    "failure mentions " + reasonPart + ": " + e.getMessage());
            check(Arrays.equals(e.raw, envelope), "failure retains original bytes");
        }
    }

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }
}
