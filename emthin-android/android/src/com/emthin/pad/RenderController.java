package com.emthin.pad;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.Executor;
import java.util.concurrent.ExecutorService;

/**
 * One serial expensive-work worker with replaceable pending work.
 *
 * <p>At most one job runs. Each lane retains only its newest pending job, so a
 * burst of edits or scroll requests replaces obsolete work before it starts.
 * Lanes are separated by purpose so a visible page requested after an
 * inspector comparison is chosen first; a newer page still replaces an older
 * page. Results carry a {@link PadSession.Owner} and are checked by the caller
 * before publication.
 */
final class RenderController {
    interface Job {
        void run();
    }

    static final int VISIBLE_PAGE = 0;
    static final int EXPLICIT = 1;
    static final int PREFETCH = 2;

    private static final class Pending {
        final String lane;
        final int priority;
        final long order;
        final Job job;

        Pending(String lane, int priority, long order, Job job) {
            this.lane = lane;
            this.priority = priority;
            this.order = order;
            this.job = job;
        }
    }

    private final Executor worker;
    private final ExecutorService ownedWorker;
    private final Map<String, Pending> pending = new LinkedHashMap<>();
    private boolean running;
    private boolean closed;
    private long nextOrder;

    RenderController(Executor worker) {
        this(worker, null);
    }

    RenderController(ExecutorService worker) {
        this(worker, worker);
    }

    private RenderController(Executor worker, ExecutorService ownedWorker) {
        this.worker = worker;
        this.ownedWorker = ownedWorker;
    }

    /** Replace the newest pending job in {@code lane}, then run if idle. */
    void submit(String lane, int priority, Job job) {
        synchronized (this) {
            if (closed) {
                return;
            }
            pending.put(lane, new Pending(lane, priority, nextOrder++, job));
            if (running) {
                return;
            }
            running = true;
        }
        worker.execute(this::drain);
    }

    /** Drop every queued job; an already running job still finishes. */
    synchronized void clearPending() {
        pending.clear();
    }

    /** Drop queued work, reject submissions and stop a worker we created. */
    synchronized void close() {
        closed = true;
        pending.clear();
        if (ownedWorker != null) {
            ownedWorker.shutdown();
        }
    }

    synchronized boolean isIdle() {
        return !running && pending.isEmpty();
    }

    private void drain() {
        while (true) {
            Pending next;
            synchronized (this) {
                next = takeNext();
                if (next == null) {
                    running = false;
                    return;
                }
            }
            next.job.run();
        }
    }

    private Pending takeNext() {
        Pending best = null;
        for (Pending candidate : pending.values()) {
            if (best == null
                    || candidate.priority < best.priority
                    || (candidate.priority == best.priority && candidate.order < best.order)) {
                best = candidate;
            }
        }
        if (best != null) {
            pending.remove(best.lane);
        }
        return best;
    }
}
