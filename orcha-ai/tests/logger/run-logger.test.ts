import { describe, it, expect, afterEach } from "vitest";
import { readFileSync, rmSync, existsSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { parseSpec } from "../../src/parser/markdown.js";
import { buildDag } from "../../src/dag/builder.js";
import { RunLogger } from "../../src/logger/run-logger.js";
import type { TaskResult } from "../../src/dag/types.js";

const fixturesDir = join(__dirname, "../fixtures");

function readFixture(name: string): string {
  return readFileSync(join(fixturesDir, name), "utf-8");
}

describe("RunLogger", () => {
  const testDir = join(tmpdir(), "orcha-test-logs");
  const cleanupDirs: string[] = [];

  afterEach(() => {
    for (const dir of cleanupDirs) {
      if (existsSync(dir)) {
        rmSync(dir, { recursive: true });
      }
    }
    cleanupDirs.length = 0;
  });

  it("creates run directory structure", () => {
    const logger = new RunLogger(testDir, "test-run");
    cleanupDirs.push(logger.runDir);

    expect(existsSync(logger.runDir)).toBe(true);
    expect(existsSync(join(logger.runDir, "tasks"))).toBe(true);
  });

  it("writes DAG json", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    const logger = new RunLogger(testDir, "test-run");
    cleanupDirs.push(logger.runDir);

    logger.writeDag(dag);

    const dagPath = join(logger.runDir, "dag.json");
    expect(existsSync(dagPath)).toBe(true);
    const dagJson = JSON.parse(readFileSync(dagPath, "utf-8"));
    expect(dagJson.roots).toEqual(["setup-project"]);
    expect(dagJson.topologicalOrder).toHaveLength(4);
  });

  it("appends task log entries as NDJSON", () => {
    const logger = new RunLogger(testDir, "test-run");
    cleanupDirs.push(logger.runDir);

    logger.appendTaskLog("my-task", { type: "start", ts: 1 });
    logger.appendTaskLog("my-task", { type: "end", ts: 2 });

    const logPath = join(logger.runDir, "tasks", "my-task.log");
    const lines = readFileSync(logPath, "utf-8").trim().split("\n");
    expect(lines).toHaveLength(2);
    expect(JSON.parse(lines[0]).type).toBe("start");
    expect(JSON.parse(lines[1]).type).toBe("end");
  });

  it("writes task result json", () => {
    const logger = new RunLogger(testDir, "test-run");
    cleanupDirs.push(logger.runDir);

    const result: TaskResult = {
      taskId: "my-task",
      success: true,
      summary: "All good",
      filesModified: ["file.ts"],
      durationMs: 500,
      costUsd: 0.02,
    };

    logger.writeTaskResult("my-task", result);

    const resultPath = join(logger.runDir, "tasks", "my-task.result.json");
    const parsed = JSON.parse(readFileSync(resultPath, "utf-8"));
    expect(parsed.success).toBe(true);
    expect(parsed.summary).toBe("All good");
  });

  it("writes run summary", () => {
    const logger = new RunLogger(testDir, "test-run");
    cleanupDirs.push(logger.runDir);

    const results: TaskResult[] = [
      {
        taskId: "a",
        success: true,
        summary: "done",
        filesModified: [],
        durationMs: 100,
        costUsd: 0.01,
      },
      {
        taskId: "b",
        success: false,
        summary: "",
        filesModified: [],
        durationMs: 50,
        costUsd: 0.005,
        error: "boom",
      },
    ];

    const summary = logger.writeSummary(results);
    expect(summary.tasks.total).toBe(2);
    expect(summary.tasks.completed).toBe(1);
    expect(summary.tasks.failed).toBe(1);
    expect(summary.totalCostUsd).toBeCloseTo(0.015);

    const summaryPath = join(logger.runDir, "summary.json");
    expect(existsSync(summaryPath)).toBe(true);
  });
});
