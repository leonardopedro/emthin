package com.emthin.pad;

/** Pure formatting for the pad's single status line (A9). */
public final class StatusModel {
    private StatusModel() {}

    /** A URI's last path segment, or "Untitled" before the first save/open. */
    public static String documentName(String lastPathSegment) {
        if (lastPathSegment == null) return "Untitled";
        String name = lastPathSegment.trim();
        return name.isEmpty() ? "Untitled" : name;
    }

    /** The durable state, with an in-flight file operation taking precedence. */
    public static String saveState(boolean recoveryReady, boolean unsaved,
                                   String activeKind, boolean activeCopy) {
        if ("save".equals(activeKind)) return activeCopy ? "saving copy" : "saving";
        if ("open".equals(activeKind)) return "opening";
        if (!recoveryReady) return "restoring";
        return unsaved ? "unsaved" : "saved";
    }

    public static String compose(String name, String state, String detail,
                                 int diagnostics) {
        StringBuilder line = new StringBuilder();
        line.append(documentName(name)).append(" · ").append(state);
        if (detail != null && !detail.trim().isEmpty()) {
            line.append(" · ").append(detail.trim());
        }
        if (diagnostics > 0) {
            line.append(" · ").append(diagnostics)
                    .append(diagnostics == 1 ? " diagnostic" : " diagnostics");
        }
        return line.toString();
    }
}
