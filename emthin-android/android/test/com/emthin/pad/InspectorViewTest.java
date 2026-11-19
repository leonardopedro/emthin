package com.emthin.pad;

/** Host checks for A14's shell-only inspector breakpoint and LRU bound. */
public final class InspectorViewTest {
    private static int checks;

    public static void main(String[] args) throws Exception {
        check(!InspectorView.columns(599, 1f), "599 dp stays in one column");
        check(InspectorView.columns(600, 1f), "600 dp uses columns");
        check(InspectorView.columns(1200, 2f), "density is applied to the breakpoint");
        check(!InspectorView.columns(1199, 2f), "density below breakpoint stays stacked");

        InspectorView.ComparisonCache cache = new InspectorView.ComparisonCache();
        for (int i = 0; i < 8; i++) {
            cache.put(InspectorView.ComparisonCache.key("a", "b", i, i + 1,
                    "c", "d", 200, 1f, 7), "row " + i);
        }
        check(cache.size() == InspectorView.MAX_COMPARISON_RESULTS,
                "cache retains only six row results");
        check(!cache.containsKey(InspectorView.ComparisonCache.key("a", "b", 0, 1,
                "c", "d", 200, 1f, 7)), "oldest row result was evicted");
        check(cache.containsKey(InspectorView.ComparisonCache.key("a", "b", 7, 8,
                "c", "d", 200, 1f, 7)), "newest row result is retained");

        cache.get(InspectorView.ComparisonCache.key("a", "b", 2, 3, "c", "d", 200, 1f, 7));
        cache.put(InspectorView.ComparisonCache.key("x", "y", 9, 10,
                "c", "d", 200, 1f, 7), "row 8");
        check(cache.containsKey(InspectorView.ComparisonCache.key("a", "b", 2, 3,
                "c", "d", 200, 1f, 7)), "accessed row survives LRU eviction");

        check(!InspectorView.ComparisonCache.key("a", "b", 1, 2, "c", "d", 200, 1f, 7)
                        .equals(InspectorView.ComparisonCache.key("a", "b", 1, 2, "c", "d",
                        200, 2f, 7)),
                "density is part of the comparison key");
        System.out.println("InspectorViewTest: " + checks + " checks passed");
    }

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }
}
