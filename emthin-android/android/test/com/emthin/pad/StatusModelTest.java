package com.emthin.pad;

public final class StatusModelTest {
    private static int checks;

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    public static void main(String[] args) {
        check(StatusModel.documentName(null).equals("Untitled"), "no URI is Untitled");
        check(StatusModel.documentName("  ").equals("Untitled"), "blank name is Untitled");
        check(StatusModel.documentName("notes.mathed").equals("notes.mathed"),
                "saved name is shown verbatim");

        check(StatusModel.saveState(true, true, null, false).equals("unsaved"),
                "unsaved document");
        check(StatusModel.saveState(true, false, null, false).equals("saved"),
                "saved document");
        check(StatusModel.saveState(false, true, null, false).equals("restoring"),
                "startup is restoring");
        check(StatusModel.saveState(true, true, "save", false).equals("saving"),
                "active save wins over unsaved");
        check(StatusModel.saveState(true, true, "save", true).equals("saving copy"),
                "copy save is labelled");
        check(StatusModel.saveState(true, true, "open", false).equals("opening"),
                "active open is labelled");

        String line = StatusModel.compose("notes.mathed", "unsaved", "Ready", 1);
        check(line.equals("notes.mathed · unsaved · Ready · 1 diagnostic"),
                "name, state, detail and singular diagnostics: " + line);
        check(StatusModel.compose("Untitled", "saving", "", 2)
                        .equals("Untitled · saving · 2 diagnostics"),
                "empty detail is omitted");
        check(!StatusModel.compose("Untitled", "saved", "Document saved", 0)
                        .contains("diagnostic"),
                "zero diagnostics is omitted");

        System.out.println("status model tests passed (" + checks + " checks)");
    }
}
