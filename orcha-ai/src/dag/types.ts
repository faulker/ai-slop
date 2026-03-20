import type { TaskDefinition } from "../parser/schema.js";

export type TaskState = "pending" | "running" | "completed" | "failed" | "cancelled";

export interface TaskResult {
  taskId: string;
  success: boolean;
  summary: string;
  filesModified: string[];
  durationMs: number;
  costUsd: number;
  error?: string;
}

export interface DagNode {
  task: TaskDefinition;
  dependsOn: string[];
  dependents: string[];
  state: TaskState;
  result?: TaskResult;
}

export interface Dag {
  nodes: Map<string, DagNode>;
  roots: string[];
  topologicalOrder: string[];
}
