package com.emthin.pad;

import android.util.AtomicFile;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.List;
import java.util.Map;

/**
 * App-private two-slot recovery storage. One owner (the recovery worker)
 * calls this class; UI-thread callers copy the returned records first.
 */
final class RecoveryStore {
    static final String SLOT_0 = "recovery-0.bin";
    static final String SLOT_1 = "recovery-1.bin";
    static final String VIEW = "view.json";

    static final class BadSlot {
        final int slot;
        final String reason;
        final boolean futureVersion;
        final byte[] raw;

        BadSlot(int slot, String reason, boolean futureVersion, byte[] raw) {
            this.slot = slot;
            this.reason = reason;
            this.futureVersion = futureVersion;
            this.raw = raw;
        }
    }

    static final class Load {
        RecoveryCodec.Record best;
        RecoveryCodec.Record older;
        final List<BadSlot> bad = new ArrayList<>();
        final List<String> errors = new ArrayList<>();
    }

    private final File directory;

    RecoveryStore(File directory) {
        this.directory = directory;
    }

    File file() {
        return directory;
    }

    Load load() {
        Load result = new Load();
        List<RecoveryCodec.Record> valid = new ArrayList<>();
        for (int slot = 0; slot < 2; slot++) {
            File file = slotFile(slot);
            if (!file.isFile()) continue;
            try {
                byte[] bytes = readBounded(file);
                valid.add(RecoveryCodec.decode(bytes));
            } catch (RecoveryCodec.DecodeException e) {
                result.bad.add(new BadSlot(slot, e.getMessage(), e.futureVersion, e.raw));
            } catch (IOException e) {
                result.errors.add("slot " + slot + ": " + e.getMessage());
            }
        }
        valid.sort(Comparator.comparingLong((RecoveryCodec.Record r) -> r.metadata.generation)
                .reversed());
        if (!valid.isEmpty()) {
            result.best = valid.get(0);
            if (valid.size() > 1) result.older = valid.get(1);
        }
        return result;
    }

    /**
     * Write to the alternate slot and read it back. A rejected existing slot
     * is quarantined before replacement; if preservation fails, the write is
     * refused.
     */
    RecoveryCodec.Record write(RecoveryCodec.Metadata metadata, byte[] snapshot)
            throws IOException, RecoveryCodec.DecodeException {
        int slot = (int) Math.floorMod(metadata.generation, 2L) == 0 ? 0 : 1;
        File target = slotFile(slot);
        if (target.isFile()) {
            byte[] existing = readBounded(target);
            try {
                RecoveryCodec.decode(existing);
            } catch (RecoveryCodec.DecodeException rejected) {
                quarantine(slot, existing);
            }
        }
        if (!directory.isDirectory() && !directory.mkdirs()) {
            throw new IOException("cannot create recovery directory");
        }
        byte[] envelope = RecoveryCodec.encode(metadata, snapshot);
        AtomicFile atomic = new AtomicFile(target);
        FileOutputStream out = atomic.startWrite();
        try {
            out.write(envelope);
            out.flush();
            out.getFD().sync();
            atomic.finishWrite(out);
        } catch (IOException e) {
            atomic.failWrite(out);
            throw e;
        }
        byte[] readBack = readBounded(target);
        RecoveryCodec.Record verified = RecoveryCodec.decode(readBack);
        if (verified.metadata.generation != metadata.generation
                || !verified.metadata.logicalFingerprint.equals(metadata.logicalFingerprint)) {
            throw new IOException("recovery read-back did not match the capture");
        }
        return verified;
    }

    void quarantine(int slot, byte[] rejected) throws IOException {
        if (rejected == null || rejected.length == 0) return;
        if (!directory.isDirectory() && !directory.mkdirs()) {
            throw new IOException("cannot create recovery directory");
        }
        String suffix = RecoveryCodec.checksumName(rejected);
        File saved = new File(directory, "recovery-" + slot + "." + suffix + ".rejected");
        if (saved.isFile()) return;
        try (FileOutputStream out = new FileOutputStream(saved, false)) {
            out.write(rejected);
            out.flush();
            out.getFD().sync();
        }
    }

    void writeView(Map<String, Object> view) throws IOException {
        if (!directory.isDirectory() && !directory.mkdirs()) {
            throw new IOException("cannot create recovery directory");
        }
        byte[] bytes = RecoveryCodec.Json.write(view).getBytes(StandardCharsets.UTF_8);
        AtomicFile atomic = new AtomicFile(new File(directory, VIEW));
        FileOutputStream out = atomic.startWrite();
        try {
            out.write(bytes);
            out.flush();
            out.getFD().sync();
            atomic.finishWrite(out);
        } catch (IOException e) {
            atomic.failWrite(out);
            throw e;
        }
    }

    @SuppressWarnings("unchecked")
    Map<String, Object> readView() {
        File file = new File(directory, VIEW);
        if (!file.isFile()) return Collections.emptyMap();
        try {
            byte[] bytes = readBounded(file);
            Object parsed = RecoveryCodec.Json.parse(new String(bytes, StandardCharsets.UTF_8));
            return parsed instanceof Map ? (Map<String, Object>) parsed : Collections.emptyMap();
        } catch (IOException | RuntimeException e) {
            return Collections.emptyMap();
        }
    }

    private File slotFile(int slot) {
        return new File(directory, slot == 0 ? SLOT_0 : SLOT_1);
    }

    private static byte[] readBounded(File file) throws IOException {
        long cap = (long) RecoveryCodec.MAX_METADATA + RecoveryCodec.MAX_SNAPSHOT + 1024;
        if (file.length() > cap) throw new IOException("recovery file exceeds bounds");
        try (FileInputStream in = new FileInputStream(file);
             ByteArrayOutputStream out = new ByteArrayOutputStream((int) file.length())) {
            byte[] buffer = new byte[16384];
            long total = 0;
            for (int n; (n = in.read(buffer)) != -1; ) {
                total += n;
                if (total > cap) throw new IOException("recovery file exceeds bounds");
                out.write(buffer, 0, n);
            }
            return out.toByteArray();
        }
    }
}
