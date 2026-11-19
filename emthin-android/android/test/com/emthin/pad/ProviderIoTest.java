package com.emthin.pad;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;

/** Host tests for A4 provider I/O, save acknowledgements and picker identity. */
public final class ProviderIoTest {
    private static int checks;

    public static void main(String[] args) throws Exception {
        readRejectsOverflow();
        readPropagatesFailure();
        strictUtf8RejectsMalformedInput();
        writeFailurePropagates();
        stampDetectsOutsideChange();
        editsDuringSaveRemainUnsaved();
        newWhilePickerOpenRejectsPicker();
        copyCompletionLeavesActiveDocumentUntouched();
        System.out.println("provider tests passed (" + checks + " checks)");
    }

    private static void readRejectsOverflow() throws Exception {
        expectIOException(() -> ProviderIo.readBounded(
                new ByteArrayInputStream(new byte[4]), 3));
        check(Arrays.equals(new byte[] {1, 2, 3},
                ProviderIo.readBounded(new ByteArrayInputStream(new byte[] {1, 2, 3}), 3)),
                "bounded read accepts an exact-limit stream");
    }

    private static void readPropagatesFailure() {
        InputStream broken = new InputStream() {
            @Override public int read() throws IOException {
                throw new IOException("provider read failed");
            }
        };
        expectIOException(() -> ProviderIo.readBounded(broken, 10));
    }

    private static void strictUtf8RejectsMalformedInput() {
        check("text".equals(decode("text".getBytes(StandardCharsets.UTF_8))),
                "strict UTF-8 accepts valid source");
        try {
            ProviderIo.decodeUtf8(new byte[] {(byte) 0xc3, 0x28});
            fail("invalid UTF-8 must not become replacement text");
        } catch (java.nio.charset.CharacterCodingException expected) {
            checks++;
        }
    }

    private static void writeFailurePropagates() {
        OutputStream broken = new OutputStream() {
            @Override public void write(int b) throws IOException {
                throw new IOException("provider write failed");
            }
        };
        expectIOException(() -> ProviderIo.writeFully(broken, new byte[] {1}));
    }

    private static void stampDetectsOutsideChange() {
        ProviderIo.Stamp opened = ProviderIo.Stamp.of("alpha".getBytes(StandardCharsets.UTF_8), 7);
        ProviderIo.Stamp same = ProviderIo.Stamp.of("alpha".getBytes(StandardCharsets.UTF_8), 99);
        ProviderIo.Stamp changed = ProviderIo.Stamp.of("beta".getBytes(StandardCharsets.UTF_8), 7);
        check(opened.known() && opened.sameContent(same), "content hash, not timestamp, matches a file");
        check(!opened.sameContent(changed), "outside edit is detected by content hash");
        check(!opened.sameContent(null), "a missing destination is treated as changed");
        check(ProviderIo.looksLikeSnapshot("loro\u0001".getBytes(StandardCharsets.ISO_8859_1)),
                "recognized Loro header is detected");
        check(!ProviderIo.looksLikeSnapshot("plain source".getBytes(StandardCharsets.UTF_8)),
                "plain source is not mistaken for a snapshot");
    }

    private static void editsDuringSaveRemainUnsaved() {
        PadSession session = new PadSession();
        PadSession.Owner save = session.capture(PadSession.FILE, "document.mathed");
        session.recordMutation();
        check(session.sameDocument(save), "save may still record the URI after a newer edit");
        check(!session.isCurrent(save), "a save completion for revision N cannot clear N+1");
    }

    private static void newWhilePickerOpenRejectsPicker() {
        PadSession session = new PadSession();
        PadSession.Owner picker = session.capture(PadSession.OPEN, "picker");
        session.beginDocument();
        check(!session.sameDocument(picker), "New rejects a picker launched for the old document");
    }

    private static void copyCompletionLeavesActiveDocumentUntouched() {
        PadSession session = new PadSession();
        PadSession.Owner save = session.capture(PadSession.FILE, "active.mathed");
        PadSession.Owner copy = session.capture(PadSession.FILE, "copy.mathed");
        session.recordMutation();
        check(session.sameDocument(save) && session.sameDocument(copy),
                "copy completion is not tied to a newer edit");
        check(!session.isCurrent(copy), "copy completion cannot claim the current revision saved");
    }

    private static String decode(byte[] bytes) {
        try {
            return ProviderIo.decodeUtf8(bytes);
        } catch (java.nio.charset.CharacterCodingException e) {
            throw new AssertionError(e);
        }
    }

    private static void expectIOException(ThrowingRunnable action) {
        try {
            action.run();
            fail("expected an IOException");
        } catch (IOException expected) {
            checks++;
        } catch (Exception e) {
            throw new AssertionError("unexpected exception", e);
        }
    }

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    private static void fail(String message) {
        throw new AssertionError(message);
    }

    private interface ThrowingRunnable {
        void run() throws Exception;
    }
}
