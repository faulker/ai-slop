import { describe, it, expect, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseSpec } from "../../src/parser/markdown.js";
import { buildDag } from "../../src/dag/builder.js";
import { DagScheduler } from "../../src/dag/scheduler.js";
import type { TaskResult } from "../../src/dag/types.js";

const fixturesDir = join(__dirname, "../fixtures");

function readFixture(name: string): string {
  return readFileSync(join(fixturesDir, name), "utf-8");
}

function makeResult(taskId: string, success: boolean): TaskResult {
  return {
    taskId,
    success,
    summary: `${taskId} ${success ? "done" : "failed"}`,
    filesModified: [],
    durationMs: 100,
    costUsd: 0.01,
    error: success ? undefined : "test error",
  };
}

describe("DagScheduler", () => {
  it("emits taskReady for root nodes on start", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    const scheduler = new DagScheduler(dag);

    const readyTasks: string[] = [];
    scheduler.on("taskReady", (id: string) => readyTasks.push(id));
    scheduler.start();

    expect(readyTasks).toEqual(["setup-project"]);
  });

  it("emits taskReady for dependents when deps complete", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    const scheduler = new DagScheduler(dag);

    const readyTasks: string[] = [];
    scheduler.on("taskReady", (id: string) => {
      readyTasks.push(id);
      scheduler.markRunning(id);
    });

    scheduler.start();
    // setup-project is ready
    expect(readyTasks).toEqual(["setup-project"]);

    // Complete setup-project
    scheduler.complete("setup-project", makeResult("setup-project", true));
    // Now build-frontend and build-backend should be ready
    expect(readyTasks).toContain("build-frontend");
    expect(readyTasks).toContain("build-backend");
  });

  it("waits for all deps before emitting taskReady", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    const scheduler = new DagScheduler(dag);

    const readyTasks: string[] = [];
    scheduler.on("taskReady", (id: string) => {
      readyTasks.push(id);
      scheduler.markRunning(id);
    });

    scheduler.start();
    scheduler.complete("setup-project", makeResult("setup-project", true));

    // Complete only build-frontend
    scheduler.complete("build-frontend", makeResult("build-frontend", true));
    // integrate should NOT be ready yet
    expect(readyTasks).not.toContain("integrate");

    // Complete build-backend
    scheduler.complete("build-backend", makeResult("build-backend", true));
    // NOW integrate should be ready
    expect(readyTasks).toContain("integrate");
  });

  it("emits allCompleted when everything is done", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    const scheduler = new DagScheduler(dag);

    const done = vi.fn();
    scheduler.on("allCompleted", done);

    scheduler.on("taskReady", (id: string) => {
      scheduler.markRunning(id);
    });

    scheduler.start();
    scheduler.complete("setup-project", makeResult("setup-project", true));
    scheduler.complete("build-frontend", makeResult("build-frontend", true));
    scheduler.complete("build-backend", makeResult("build-backend", true));
    scheduler.complete("integrate", makeResult("integrate", true));

    expect(done).toHaveBeenCalledOnce();
  });

  it("cascades failure to transitive dependents", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    const scheduler = new DagScheduler(dag);

    const failedTasks: string[] = [];
    scheduler.on("taskFailed", (id: string) => failedTasks.push(id));
    scheduler.on("taskReady", (id: string) => scheduler.markRunning(id));

    scheduler.start();
    // Fail setup-project
    scheduler.complete("setup-project", makeResult("setup-project", false));

    // All downstream should be cancelled
    expect(failedTasks).toContain("setup-project");
    expect(failedTasks).toContain("build-frontend");
    expect(failedTasks).toContain("build-backend");
    expect(failedTasks).toContain("integrate");
  });

  it("respects maxConcurrency", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    // Set concurrency to 1
    const scheduler = new DagScheduler(dag, 1);

    const readyTasks: string[] = [];
    scheduler.on("taskReady", (id: string) => {
      readyTasks.push(id);
      scheduler.markRunning(id);
    });

    scheduler.start();
    scheduler.complete("setup-project", makeResult("setup-project", true));

    // With concurrency=1, only one of the parallel tasks should be emitted
    // The second should be emitted after the first completes
    const parallelReady = readyTasks.filter(
      (id) => id === "build-frontend" || id === "build-backend"
    );
    expect(parallelReady).toHaveLength(1);

    // Complete the first parallel task
    scheduler.complete(parallelReady[0], makeResult(parallelReady[0], true));
    const nowReady = readyTasks.filter(
      (id) => id === "build-frontend" || id === "build-backend"
    );
    expect(nowReady).toHaveLength(2);
  });

  it("handles single-task DAG", () => {
    const spec = parseSpec(readFixture("minimal-plan.md"));
    const dag = buildDag(spec);
    const scheduler = new DagScheduler(dag);

    const done = vi.fn();
    scheduler.on("allCompleted", done);
    scheduler.on("taskReady", (id: string) => scheduler.markRunning(id));

    scheduler.start();
    scheduler.complete("only-task", makeResult("only-task", true));

    expect(done).toHaveBeenCalledOnce();
  });
});
