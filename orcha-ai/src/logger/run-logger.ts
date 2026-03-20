import { mkdirSync, writeFileSync, appendFileSync } from "node:fs";
import { join } from "node:path";
import type { Dag, TaskResult } from "../dag/types.js";

export interface RunSummary {
  name: string;
  startedAt: string;
  completedAt: string;
  durationMs: number;
  totalCostUsd: number;
  tasks: {
    total: number;
    completed: number;
    failed: number;
    cancelled: number;
  };
  results: TaskResult[];
}

export class RunLogger {
  readonly runDir: string;
  private tasksDir: string;
  private startTime: number;
  private name: string;

  constructor(baseDir: string, name: string) {
    this.name = name;
    this.startTime = Date.now();
    const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
    this.runDir = join(baseDir, `${timestamp}-${name}`);
    this.tasksDir = join(this.runDir, "tasks");
    mkdirSync(this.tasksDir, { recursive: true });
  }

  writeDag(dag: Dag): void {
    const serializable = {
      roots: dag.roots,
      topologicalOrder: dag.topologicalOrder,
      nodes: Object.fromEntries(
        [...dag.nodes.entries()].map(([id, node]) => [
          id,
          {
            dependsOn: node.dependsOn,
            dependents: node.dependents,
            state: node.state,
          },
        ])
      ),
    };
    writeFileSync(
      join(this.runDir, "dag.json"),
      JSON.stringify(serializable, null, 2)
    );
  }

  appendTaskLog(taskId: string, entry: Record<string, unknown>): void {
    const logPath = join(this.tasksDir, `${taskId}.log`);
    appendFileSync(logPath, JSON.stringify(entry) + "\n");
  }

  writeTaskResult(taskId: string, result: TaskResult): void {
    writeFileSync(
      join(this.tasksDir, `${taskId}.result.json`),
      JSON.stringify(result, null, 2)
    );
  }

  writeSummary(results: TaskResult[]): RunSummary {
    const completedAt = new Date().toISOString();
    const summary: RunSummary = {
      name: this.name,
      startedAt: new Date(this.startTime).toISOString(),
      completedAt,
      durationMs: Date.now() - this.startTime,
      totalCostUsd: results.reduce((sum, r) => sum + r.costUsd, 0),
      tasks: {
        total: results.length,
        completed: results.filter((r) => r.success).length,
        failed: results.filter((r) => !r.success && r.error && !r.error.startsWith("Upstream")).length,
        cancelled: results.filter((r) => !r.success && r.error?.startsWith("Upstream")).length,
      },
      results,
    };
    writeFileSync(
      join(this.runDir, "summary.json"),
      JSON.stringify(summary, null, 2)
    );
    return summary;
  }
}
