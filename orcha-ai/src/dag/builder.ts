import type { SpecFile } from "../parser/schema.js";
import type { Dag, DagNode } from "./types.js";

export function buildDag(spec: SpecFile): Dag {
  const nodes = new Map<string, DagNode>();

  // Create nodes
  for (const task of spec.tasks) {
    nodes.set(task.id, {
      task,
      dependsOn: [...task.metadata.dependsOn],
      dependents: [],
      state: "pending",
    });
  }

  // Compute reverse edges (dependents)
  for (const task of spec.tasks) {
    for (const dep of task.metadata.dependsOn) {
      const depNode = nodes.get(dep);
      if (depNode) {
        depNode.dependents.push(task.id);
      }
    }
  }

  // Find roots (no dependencies)
  const roots = spec.tasks
    .filter((t) => t.metadata.dependsOn.length === 0)
    .map((t) => t.id);

  // Topological sort via Kahn's algorithm
  const topologicalOrder = kahnSort(spec);

  return { nodes, roots, topologicalOrder };
}

function kahnSort(spec: SpecFile): string[] {
  const inDegree = new Map<string, number>();
  const adjacency = new Map<string, string[]>();

  for (const task of spec.tasks) {
    inDegree.set(task.id, 0);
    adjacency.set(task.id, []);
  }

  for (const task of spec.tasks) {
    for (const dep of task.metadata.dependsOn) {
      adjacency.get(dep)?.push(task.id);
      inDegree.set(task.id, (inDegree.get(task.id) ?? 0) + 1);
    }
  }

  const queue: string[] = [];
  for (const [id, degree] of inDegree) {
    if (degree === 0) queue.push(id);
  }

  const order: string[] = [];
  while (queue.length > 0) {
    const node = queue.shift()!;
    order.push(node);
    for (const dependent of adjacency.get(node) ?? []) {
      const newDegree = (inDegree.get(dependent) ?? 1) - 1;
      inDegree.set(dependent, newDegree);
      if (newDegree === 0) queue.push(dependent);
    }
  }

  return order;
}
