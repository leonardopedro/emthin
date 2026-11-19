package com.emthin.pad;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Shell-only inspector layout and comparison retention. Nothing here touches
 * the document or Typst geometry.
 */
public final class InspectorView {
    public static final int BREAKPOINT_DP = 600;
    public static final int MAX_COMPARISON_RESULTS = 6;

    private InspectorView() {}

    /** True when source/target and comparison cards use aligned columns. */
    public static boolean columns(int widthPx, float density) {
        if (density <= 0f) density = 1f;
        return widthPx / density >= BREAKPOINT_DP;
    }

    /** Bounded, access-ordered row-result cache; never more than six rasters. */
    public static final class ComparisonCache extends LinkedHashMap<String, Object> {
        private static final long serialVersionUID = 1L;

        public ComparisonCache() {
            super(8, 0.75f, true);
        }

        @Override protected boolean removeEldestEntry(Map.Entry<String, Object> eldest) {
            return size() > MAX_COMPARISON_RESULTS;
        }

        public static String key(String startId, String endId, int targetStart, int targetEnd,
                                 String excludeStart, String excludeEnd, int width, float scale,
                                 int textHash) {
            return startId + ":" + endId + ":" + targetStart + ":" + targetEnd
                    + ":" + excludeStart + ":" + excludeEnd + ":" + width + ":" + scale
                    + ":" + textHash;
        }
    }
}
