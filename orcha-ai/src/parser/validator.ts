import type { SpecFile } from "./schema.js";

export interface ValidationError {
  type: "missing_dependency" | "cycle" | "duplicate_id";
  message: string;
}

export function validateSpec(spec: SpecFile): ValidationError[] {
  const errors: ValidationError[] = [];
  const taskIds = new Set(spec.tasks.map((t) => t.id));

  // Check for duplicate IDs
  const seen = new Set<string>();
  for (const task of spec.tasks) {
    if (seen.has(task.id)) {
      errors.push({
        type: "duplicate_id",
        message: `Duplicate task ID: "${task.id}"`,
      });
    }
    seen.add(task.id);
  }

  // Check referential integrity
  for (const task of spec.tasks) {
    for (const dep of task.metadata.dependsOn) {
      if (!taskIds.has(dep)) {
        errors.push({
          type: "missing_dependency",
          message: `Task "${task.id}" depends on unknown task "${dep}"`,
        });
      }
    }
  }

  // Cycle detection via Kahn's algorithm
  const cycleError = detectCycles(spec);
  if (cycleError) {
    errors.push(cycleError);
  }

  return errors;
}

function detectCycles(spec: SpecFile): ValidationError | null {
  const inDegree = new Map<string, number>();
  const adjacency = new Map<string, string[]>();

  for (const task of spec.tasks) {
    inDegree.set(task.id, 0);
    adjacency.set(task.id, []);
  }

  for (const task of spec.tasks) {
    for (const dep of task.metadata.dependsOn) {
      if (adjacency.has(dep)) {
        adjacency.get(dep)!.push(task.id);
        inDegree.set(task.id, (inDegree.get(task.id) ?? 0) + 1);
      }
    }
  }

  const queue: string[] = [];
  for (const [id, degree] of inDegree) {
    if (degree === 0) queue.push(id);
  }

  let processed = 0;
  while (queue.length > 0) {
    const node = queue.shift()!;
    processed++;
    for (const dependent of adjacency.get(node) ?? []) {
      const newDegree = (inDegree.get(dependent) ?? 1) - 1;
      inDegree.set(dependent, newDegree);
      if (newDegree === 0) queue.push(dependent);
    }
  }

  if (processed < spec.tasks.length) {
    const cycleNodes = [...inDegree.entries()]
      .filter(([, deg]) => deg > 0)
      .map(([id]) => id);
    return {
      type: "cycle",
      message: `Dependency cycle detected involving: ${cycleNodes.join(", ")}`,
    };
  }

  return null;
}
