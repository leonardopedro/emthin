package com.emthin.pad;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;

/**
 * Stream and content-identity helpers shared by provider opens and saves.
 *
 * <p>This class is deliberately free of Android imports so the host test lane
 * can exercise truncation, invalid UTF-8 and failed writes with fake streams.
 */
final class ProviderIo {
    static final int MAX_DOCUMENT_BYTES = 32 * 1024 * 1024;

    private ProviderIo() {}

    /** Read at most {@code max} bytes; a longer stream is an error, not truncation. */
    static byte[] readBounded(InputStream input, int max) throws IOException {
        if (input == null) throw new IOException("No document stream");
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buffer = new byte[16384];
        for (int n; (n = input.read(buffer)) != -1; ) {
            if (n == 0) continue;
            if (out.size() + n > max) throw new IOException("Document exceeds 32 MB");
            out.write(buffer, 0, n);
        }
        return out.toByteArray();
    }

    /** Strict UTF-8: malformed source is an error, never replacement characters. */
    static String decodeUtf8(byte[] bytes) throws CharacterCodingException {
        return StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(bytes)).toString();
    }

    static void writeFully(OutputStream out, byte[] bytes) throws IOException {
        if (out == null) throw new IOException("No document stream");
        out.write(bytes);
        out.flush();
    }

    /** Loro snapshots begin with the four-byte "loro" encoding magic. */
    static boolean looksLikeSnapshot(byte[] bytes) {
        return bytes != null && bytes.length >= 4 && bytes[0] == 'l'
                && bytes[1] == 'o' && bytes[2] == 'r' && bytes[3] == 'o';
    }

    static String sha256(byte[] bytes) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            byte[] hash = digest.digest(bytes);
            StringBuilder hex = new StringBuilder(hash.length * 2);
            for (byte b : hash) hex.append(String.format(java.util.Locale.ROOT, "%02x", b & 0xff));
            return hex.toString();
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException("SHA-256 unavailable", e);
        }
    }

    /** The provider content identity remembered after Open or a successful Save. */
    static final class Stamp {
        final String sha256;
        final long size;
        final long modified;

        Stamp(String sha256, long size, long modified) {
            this.sha256 = sha256 == null ? "" : sha256;
            this.size = size;
            this.modified = modified;
        }

        static Stamp of(byte[] bytes, long modified) {
            return new Stamp(sha256(bytes), bytes.length, modified);
        }

        boolean known() {
            return !sha256.isEmpty();
        }

        boolean sameContent(Stamp other) {
            return other != null && known() && sha256.equals(other.sha256);
        }
    }
}
