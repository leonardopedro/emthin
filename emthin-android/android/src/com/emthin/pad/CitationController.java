package com.emthin.pad;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;

/**
 * Presentation and path state for the citation dialog.
 *
 * <p>This owns no document text and never mutates the model. Style/locale
 * changes only invalidate detail formatting; path entries identify one cite
 * by its scope and source range so nested repeats can stop without blocking
 * source access.
 */
public final class CitationController {
    public static final int MAX_DEPTH = 16;

    public static final class Path {
        public final String scope;
        public final int number;
        public final int start;
        public final int end;
        public final String label;

        Path(String scope, int number, int start, int end, String label) {
            this.scope = scope;
            this.number = number;
            this.start = start;
            this.end = end;
            this.label = label;
        }
    }

    private String[] styles = new String[0];
    private String[] locales = new String[0];
    private String defaultStyle = "";
    private String defaultLocale = "";
    private String style;
    private String locale;
    private String filter = "";
    private int version;

    public void setChoices(String[] styles, String[] locales,
                           String defaultStyle, String defaultLocale) {
        this.styles = styles == null ? new String[0] : styles.clone();
        this.locales = locales == null ? new String[0] : locales.clone();
        this.defaultStyle = defaultStyle == null ? "" : defaultStyle;
        this.defaultLocale = defaultLocale == null ? "" : defaultLocale;
    }

    public String[] styles() { return styles.clone(); }
    public String[] locales() { return locales.clone(); }
    public String defaultStyle() { return defaultStyle; }
    public String defaultLocale() { return defaultLocale; }

    /** The style passed to the formatter; null means the document's own style. */
    public String style() { return style; }
    /** The locale passed to the formatter; null means the shared default. */
    public String locale() { return locale; }

    public String styleLabel() {
        return style == null ? "Document default" : style;
    }

    public String localeLabel() {
        return locale == null ? "Default (" + defaultLocale + ")" : locale;
    }

    /** Returns false for a name outside the shared list. */
    public boolean setStyle(String name) {
        if (name != null && !contains(styles, name)) return false;
        if (!equal(style, name)) {
            style = name;
            version++;
        }
        return true;
    }

    public boolean setLocale(String name) {
        if (name != null && !contains(locales, name)) return false;
        if (!equal(locale, name)) {
            locale = name;
            version++;
        }
        return true;
    }

    public void setSavedFormat(String style, String locale) {
        this.style = style != null && contains(styles, style) ? style : null;
        this.locale = locale != null && contains(locales, locale) ? locale : null;
        version++;
    }

    public boolean resetFormat() {
        if (style == null && locale == null) return false;
        style = null;
        locale = null;
        version++;
        return true;
    }

    /** Monotonic formatting generation; callbacks from an older one are stale. */
    public int version() { return version; }

    public void setFilter(String value) {
        filter = value == null ? "" : value.trim();
    }

    public String filter() { return filter; }

    public boolean matches(String number, String label, String snippet,
                           String keys, boolean resolved) {
        return matchesFilter(filter, number, label, snippet, keys, resolved);
    }

    public static boolean matchesFilter(String filter, String number, String label,
                                        String snippet, String keys, boolean resolved) {
        if (filter == null || filter.trim().isEmpty()) return true;
        String needle = filter.trim().toLowerCase(Locale.ROOT);
        String status = resolved ? "resolved" : "unresolved";
        return containsIgnoreCase(number, needle)
                || containsIgnoreCase(label, needle)
                || containsIgnoreCase(snippet, needle)
                || containsIgnoreCase(keys, needle)
                || containsIgnoreCase(status, needle);
    }

    /** Append a cite to a path; null means a cycle or the depth limit. */
    public List<Path> append(List<Path> parent, String scope, int number,
                             int start, int end, String label) {
        if (parent == null || parent.size() >= MAX_DEPTH) return null;
        if (isCycle(parent, scope, number, start, end)) return null;
        List<Path> next = new ArrayList<>(parent);
        next.add(new Path(scope, number, start, end, label == null ? "" : label));
        return Collections.unmodifiableList(next);
    }

    public static boolean isCycle(List<Path> path, String scope, int number,
                                  int start, int end) {
        if (path == null) return false;
        for (Path item : path) {
            if (item.number == number && item.start == start && item.end == end
                    && item.scope.equals(scope)) return true;
        }
        return false;
    }

    public static String breadcrumb(List<Path> path) {            StringBuilder text = new StringBuilder();
        for (Path item : path) {
            if (text.length() > 0) text.append(" › ");
            text.append('[').append(item.number).append(']');
            String label = item.label == null ? "" : item.label.trim().replace('\n', ' ');
            label = label.replaceFirst("^\\[\\d+\\]\\s*", "");
            if (!label.isEmpty()) {
                if (label.length() > 48) label = label.substring(0, 48) + "…";
                text.append(' ').append(label);
            }
        }
        return text.length() == 0 ? "Document" : text.toString();
    }

    /** The root source range, or null when the path is empty. */
    public static int[] rootSource(List<Path> path) {
        if (path == null || path.isEmpty()) return null;
        Path root = path.get(0);
        return new int[] {root.start, root.end};
    }

    private static boolean contains(String[] values, String name) {
        for (String value : values) {
            if (value.equals(name)) return true;
        }
        return false;
    }

    private static boolean equal(String a, String b) {
        return a == null ? b == null : a.equals(b);
    }

    private static boolean containsIgnoreCase(String value, String lowerNeedle) {
        return value != null && value.toLowerCase(Locale.ROOT).contains(lowerNeedle);
    }
}
