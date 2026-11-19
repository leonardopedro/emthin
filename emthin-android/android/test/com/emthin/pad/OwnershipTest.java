package com.emthin.pad;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Deque;
import java.util.List;

/**
 * Host tests for A2's asynchronous ownership. Run with {@code test-java.sh}.
 * No Android classes are involved: the coordinators are plain Java.
 */
public final class OwnershipTest {
    private static int checks;

    public static void main(String[] args) {
        laterResultDoesNotChangeNewerCapture();
        newOrOpenRejectsOldDocumentWork();
        disposeRejectsEverything();
        marksOnlyEditInvalidatesContentWork();
        fileMetadataMaySurviveNewerEdits();
        latestLaneRejectsSupersededInspectorWork();
        latestPendingPageReplacesQueuedPage();
        activeJobKeepsOneNewestPending();
        pagePriorityBeatsPendingInspectorWork();
        idleControllerSchedulesNothing();
        System.out.println("ownership tests passed (" + checks + " checks)");
    }

    private static void laterResultDoesNotChangeNewerCapture() {
        PadSession session = new PadSession();
        PadSession.Owner first = session.capture(PadSession.RENDER, "p1");
        session.recordMutation();
        PadSession.Owner second = session.capture(PadSession.RENDER, "p2");

        List<String> published = new ArrayList<>();
        // Deliberately deliver the newer result first, then the older one.
        if (session.isCurrent(second)) published.add("second");
        if (session.isCurrent(first)) published.add("first");
        check(published.equals(Arrays.asList("second")),
                "out-of-order delivery publishes only the current capture");
    }

    private static void newOrOpenRejectsOldDocumentWork() {
        PadSession session = new PadSession();
        PadSession.Owner old = session.capture(PadSession.RENDER, "old");
        session.beginDocument();
        PadSession.Owner fresh = session.capture(PadSession.RENDER, "fresh");
        check(!session.isCurrent(old), "New/Open rejects the old document capture");
        check(!session.sameDocument(old), "New/Open rejects old file metadata");
        check(session.isCurrent(fresh), "New/Open accepts its new capture");
    }

    private static void disposeRejectsEverything() {
        PadSession session = new PadSession();
        PadSession.Owner work = session.capture(PadSession.INSPECTOR, "target");
        session.dispose();
        check(!session.isCurrent(work), "disposed session rejects results");
        check(!session.sameDocument(work), "disposed session rejects file metadata");
    }

    private static void marksOnlyEditInvalidatesContentWork() {
        PadSession session = new PadSession();
        PadSession.Owner work = session.capture(PadSession.RENDER, "same-text");
        // A mark/modifier change does not alter text revision, but it does
        // alter the content capture's mutation sequence.
        session.recordMutation();
        check(!session.isCurrent(work), "marks-only mutation invalidates content work");
    }

    private static void fileMetadataMaySurviveNewerEdits() {
        PadSession session = new PadSession();
        PadSession.Owner save = session.capture(PadSession.FILE, "doc");
        session.recordMutation();
        check(session.sameDocument(save), "an older save may still record its URI");
        check(!session.isCurrent(save), "an older save is not a current capture");
    }

    private static void latestLaneRejectsSupersededInspectorWork() {
        PadSession session = new PadSession();
        PadSession.Owner first = session.capture(PadSession.INSPECTOR, "a");
        PadSession.Owner second = session.capture(PadSession.INSPECTOR, "b");
        check(!session.isLatest(first), "superseded inspector result is rejected");
        check(session.isLatest(second), "newest inspector result is accepted");
        session.invalidate(PadSession.INSPECTOR);
        check(!session.isLatest(second), "invalidate rejects the active inspector result");
    }

    private static void latestPendingPageReplacesQueuedPage() {
        ManualExecutor executor = new ManualExecutor();
        RenderController controller = new RenderController(executor);
        List<String> ran = new ArrayList<>();
        controller.submit("page", RenderController.VISIBLE_PAGE, () -> ran.add("old"));
        controller.submit("page", RenderController.VISIBLE_PAGE, () -> ran.add("new"));
        executor.runNext();
        check(ran.equals(Arrays.asList("new")),
                "replaceable pending page drops obsolete queued work");
        check(controller.isIdle(), "controller becomes idle after draining");
    }

    private static void activeJobKeepsOneNewestPending() {
        ManualExecutor executor = new ManualExecutor();
        RenderController controller = new RenderController(executor);
        List<String> ran = new ArrayList<>();
        controller.submit("page", RenderController.VISIBLE_PAGE, () -> {
            ran.add("active");
            controller.submit("page", RenderController.VISIBLE_PAGE, () -> ran.add("middle"));
            controller.submit("page", RenderController.VISIBLE_PAGE, () -> ran.add("latest"));
        });
        executor.runNext();
        check(ran.equals(Arrays.asList("active", "latest")),
                "one active job plus one replaceable latest pending job");
    }

    private static void pagePriorityBeatsPendingInspectorWork() {
        ManualExecutor executor = new ManualExecutor();
        RenderController controller = new RenderController(executor);
        List<String> ran = new ArrayList<>();
        controller.submit("page", RenderController.VISIBLE_PAGE, () -> {
            ran.add("active");
            controller.submit("inspector", RenderController.EXPLICIT, () -> ran.add("inspector"));
            controller.submit("page", RenderController.VISIBLE_PAGE, () -> ran.add("page"));
        });
        executor.runNext();
        check(ran.equals(Arrays.asList("active", "page", "inspector")),
                "visible page work is chosen before pending inspector work");
    }

    private static void idleControllerSchedulesNothing() {
        ManualExecutor executor = new ManualExecutor();
        RenderController controller = new RenderController(executor);
        List<String> ran = new ArrayList<>();
        controller.submit("page", RenderController.VISIBLE_PAGE, () -> ran.add("once"));
        executor.runNext();
        executor.runNext();
        check(ran.equals(Arrays.asList("once")), "finished work does not reschedule itself");
        check(executor.queued() == 0, "idle controller leaves no queued task");
    }

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) {
            throw new AssertionError(message);
        }
    }

    /** A controllable worker: tests decide exactly when the drain runs. */
    private static final class ManualExecutor implements java.util.concurrent.Executor {
        private final Deque<Runnable> tasks = new ArrayDeque<>();

        @Override public void execute(Runnable command) {
            tasks.addLast(command);
        }

        void runNext() {
            Runnable next = tasks.pollFirst();
            if (next != null) {
                next.run();
            }
        }

        int queued() {
            return tasks.size();
        }
    }
}
