import { EventEmitter } from "node:events";
import type { Dag, TaskResult } from "./types.js";

export interface SchedulerEvents {
  taskReady: (taskId: string) => void;
  taskCompleted: (taskId: string, result: TaskResult) => void;
  taskFailed: (taskId: string, result: TaskResult) => void;
  allCompleted: () => void;
}

export class DagScheduler extends EventEmitter {
  private dag: Dag;
  private maxConcurrency: number;
  private runningCount = 0;
  private pendingReady: string[] = [];

  constructor(dag: Dag, maxConcurrency = 4) {
    super();
    this.dag = dag;
    this.maxConcurrency = maxConcurrency;
  }

  start(): void {
    for (const root of this.dag.roots) {
      this.enqueueReady(root);
    }
    this.drainQueue();
  }

  markRunning(taskId: string): void {
    const node = this.dag.nodes.get(taskId);
    if (node) {
      node.state = "running";
      this.runningCount++;
    }
  }

  complete(taskId: string, result: TaskResult): void {
    const node = this.dag.nodes.get(taskId);
    if (!node) return;

    node.state = result.success ? "completed" : "failed";
    node.result = result;
    this.runningCount--;

    if (result.success) {
      this.emit("taskCompleted", taskId, result);
      // Check if dependents are unblocked
      for (const depId of node.dependents) {
        if (this.allDepsComplete(depId)) {
          this.enqueueReady(depId);
        }
      }
    } else {
      this.emit("taskFailed", taskId, result);
      this.cascadeFailure(taskId);
    }

    this.drainQueue();

    if (this.isDone()) {
      this.emit("allCompleted");
    }
  }

  private enqueueReady(taskId: string): void {
    this.pendingReady.push(taskId);
  }

  private drainQueue(): void {
    while (this.pendingReady.length > 0 && this.runningCount < this.maxConcurrency) {
      const taskId = this.pendingReady.shift()!;
      this.emit("taskReady", taskId);
    }
  }

  private allDepsComplete(taskId: string): boolean {
    const node = this.dag.nodes.get(taskId);
    if (!node) return false;
    return node.dependsOn.every((dep) => {
      const depNode = this.dag.nodes.get(dep);
      return depNode?.state === "completed";
    });
  }

  private cascadeFailure(failedId: string): void {
    const node = this.dag.nodes.get(failedId);
    if (!node) return;

    for (const depId of node.dependents) {
      const depNode = this.dag.nodes.get(depId);
      if (depNode && depNode.state === "pending") {
        depNode.state = "cancelled";
        depNode.result = {
          taskId: depId,
          success: false,
          summary: `Cancelled: upstream task "${failedId}" failed`,
          filesModified: [],
          durationMs: 0,
          costUsd: 0,
          error: `Upstream dependency "${failedId}" failed`,
        };
        this.emit("taskFailed", depId, depNode.result);
        this.cascadeFailure(depId);
      }
    }
  }

  private isDone(): boolean {
    for (const node of this.dag.nodes.values()) {
      if (node.state === "pending" || node.state === "running") {
        return false;
      }
    }
    return this.pendingReady.length === 0;
  }

  getState(): Map<string, string> {
    const state = new Map<string, string>();
    for (const [id, node] of this.dag.nodes) {
      state.set(id, node.state);
    }
    return state;
  }
}
