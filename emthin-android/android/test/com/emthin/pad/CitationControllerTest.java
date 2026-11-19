package com.emthin.pad;

import java.util.ArrayList;
import java.util.List;

/** Host checks for A13 citation choices, paths, cycles, depth and filtering. */
public final class CitationControllerTest {
    private static int checks;

    public static void main(String[] args) {
        CitationController c = new CitationController();
        c.setChoices(new String[] {"apa", "mla"}, new String[] {"en-US", "de-DE"},
                "apa", "en-US");
        check(c.style() == null && c.styleLabel().equals("Document default"),
                "default style follows the document");
        check(c.locale() == null && c.localeLabel().equals("Default (en-US)"),
                "default locale is labelled");
        check(!c.setStyle("bogus"), "unknown style is rejected");
        check(!c.setLocale("xx-XX"), "unknown locale is rejected");
        int before = c.version();
        check(c.setStyle("mla") && c.version() == before + 1, "style change bumps version");
        before = c.version();
        check(c.setStyle("mla") && c.version() == before, "repeated style does not bump");
        check(c.setLocale("de-DE") && "de-DE".equals(c.localeLabel()), "locale selected");
        check(c.resetFormat() && c.style() == null && c.locale() == null,
                "reset restores document/default");
        check(!c.resetFormat(), "second reset is a no-op");
        c.setSavedFormat("mla", "ja-JP");
        check("mla".equals(c.style()) && c.locale() == null,
                "invalid saved locale is ignored");
        c.setSavedFormat(null, null);

        List<CitationController.Path> path = new ArrayList<>();
        path = c.append(path, "root", 1, 10, 20, "First cite");
        path = c.append(path, "body", 1, 2, 8, "Nested cite");
        check(path != null && path.size() == 2, "nested path appends");
        check(CitationController.breadcrumb(path).equals("[1] First cite › [1] Nested cite"),
                "breadcrumb shows the chain");
        check(c.append(path, "body", 1, 2, 8, "again") == null,
                "same reference identity is a cycle");
        check(c.append(path, "body", 2, 2, 8, "other key") != null,
                "same range with another number is not a cycle");
        check(c.append(path, "body", 1, 3, 8, "shifted") != null,
                "a moved source range is not the same reference");

        List<CitationController.Path> deep = new ArrayList<>();
        for (int i = 0; i < CitationController.MAX_DEPTH; i++) {
            deep = c.append(deep, "scope" + i, i + 1, i, i + 1, "level " + i);
            if (deep == null) break;
        }
        check(deep != null && deep.size() == 16, "sixteen nested levels are allowed");
        check(c.append(deep, "scope16", 17, 0, 1, "too deep") == null,
                "the seventeenth level is refused");
        int[] source = CitationController.rootSource(deep);
        check(source != null && source[0] == 0 && source[1] == 1,
                "root source remains available at the depth limit");

        c.setFilter("smi");
        check(c.matches("4", "Smith 2020", "", "smith2020", true),
                "filter matches label, key or number case-insensitively");
        check(!c.matches("5", "Jones", "nothing", "jones", false),
                "filter rejects unrelated rows");
        c.setFilter("unres");
        check(c.matches("5", "Jones", "", "jones", false), "filter matches status");
        c.setFilter("");
        check(c.matches("5", "Jones", "", "jones", false), "empty filter keeps all rows");

        System.out.println("CitationControllerTest: " + checks + " checks passed");
    }

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }
}
