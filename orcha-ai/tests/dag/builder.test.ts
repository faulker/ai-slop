import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseSpec } from "../../src/parser/markdown.js";
import { buildDag } from "../../src/dag/builder.js";

const fixturesDir = join(__dirname, "../fixtures");

function readFixture(name: string): string {
  return readFileSync(join(fixturesDir, name), "utf-8");
}

describe("buildDag", () => {
  it("identifies root nodes correctly", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    expect(dag.roots).toEqual(["setup-project"]);
  });

  it("computes reverse edges (dependents)", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);

    const setupNode = dag.nodes.get("setup-project")!;
    expect(setupNode.dependents).toContain("build-frontend");
    expect(setupNode.dependents).toContain("build-backend");

    const frontendNode = dag.nodes.get("build-frontend")!;
    expect(frontendNode.dependents).toContain("integrate");
  });

  it("produces valid topological order", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);

    const order = dag.topologicalOrder;
    const indexOf = (id: string) => order.indexOf(id);

    // setup-project must come before its dependents
    expect(indexOf("setup-project")).toBeLessThan(indexOf("build-frontend"));
    expect(indexOf("setup-project")).toBeLessThan(indexOf("build-backend"));
    // integrate must come after both
    expect(indexOf("build-frontend")).toBeLessThan(indexOf("integrate"));
    expect(indexOf("build-backend")).toBeLessThan(indexOf("integrate"));
  });

  it("handles single-node DAG", () => {
    const spec = parseSpec(readFixture("minimal-plan.md"));
    const dag = buildDag(spec);
    expect(dag.roots).toEqual(["only-task"]);
    expect(dag.topologicalOrder).toEqual(["only-task"]);
    expect(dag.nodes.get("only-task")!.dependents).toEqual([]);
  });

  it("initializes all nodes as pending", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const dag = buildDag(spec);
    for (const node of dag.nodes.values()) {
      expect(node.state).toBe("pending");
    }
  });
});
