import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseSpec } from "../../src/parser/markdown.js";
import { validateSpec } from "../../src/parser/validator.js";

const fixturesDir = join(__dirname, "../fixtures");

function readFixture(name: string): string {
  return readFileSync(join(fixturesDir, name), "utf-8");
}

describe("validateSpec", () => {
  it("returns no errors for a valid plan", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    const errors = validateSpec(spec);
    expect(errors).toHaveLength(0);
  });

  it("returns no errors for a minimal plan", () => {
    const spec = parseSpec(readFixture("minimal-plan.md"));
    const errors = validateSpec(spec);
    expect(errors).toHaveLength(0);
  });

  it("detects missing dependencies", () => {
    const spec = parseSpec(readFixture("missing-dep-plan.md"));
    const errors = validateSpec(spec);
    const missingDep = errors.find((e) => e.type === "missing_dependency");
    expect(missingDep).toBeDefined();
    expect(missingDep!.message).toContain("nonexistent-task");
  });

  it("detects cycles", () => {
    const spec = parseSpec(readFixture("cyclic-plan.md"));
    const errors = validateSpec(spec);
    const cycle = errors.find((e) => e.type === "cycle");
    expect(cycle).toBeDefined();
    expect(cycle!.message).toContain("cycle");
  });

  it("detects duplicate task IDs", () => {
    const spec = parseSpec(readFixture("valid-plan.md"));
    // Manually add a duplicate
    spec.tasks.push({ ...spec.tasks[0] });
    const errors = validateSpec(spec);
    const dup = errors.find((e) => e.type === "duplicate_id");
    expect(dup).toBeDefined();
  });
});
