import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseFrontmatter, parseTaskSections, parseSpec } from "../../src/parser/markdown.js";

const fixturesDir = join(__dirname, "../fixtures");

function readFixture(name: string): string {
  return readFileSync(join(fixturesDir, name), "utf-8");
}

describe("parseFrontmatter", () => {
  it("parses valid frontmatter with all fields", () => {
    const content = readFixture("valid-plan.md");
    const fm = parseFrontmatter(content);
    expect(fm.name).toBe("test-project");
    expect(fm.goal).toBe("Build a test application");
    expect(fm.model).toBe("claude-sonnet-4-20250514");
    expect(fm.maxBudgetUsd).toBe(5.0);
  });

  it("parses frontmatter with only required fields", () => {
    const content = readFixture("minimal-plan.md");
    const fm = parseFrontmatter(content);
    expect(fm.name).toBe("minimal");
    expect(fm.goal).toBe("Single task plan");
    expect(fm.model).toBeUndefined();
    expect(fm.maxBudgetUsd).toBeUndefined();
  });

  it("throws on missing frontmatter", () => {
    expect(() => parseFrontmatter("# No frontmatter")).toThrow("Missing YAML frontmatter");
  });

  it("throws on missing required fields", () => {
    const content = "---\nname: \"test\"\n---\n";
    expect(() => parseFrontmatter(content)).toThrow();
  });
});

describe("parseTaskSections", () => {
  it("parses all task sections from valid plan", () => {
    const content = readFixture("valid-plan.md");
    const tasks = parseTaskSections(content);
    expect(tasks).toHaveLength(4);
    expect(tasks.map((t) => t.id)).toEqual([
      "setup-project",
      "build-frontend",
      "build-backend",
      "integrate",
    ]);
  });

  it("extracts metadata correctly", () => {
    const content = readFixture("valid-plan.md");
    const tasks = parseTaskSections(content);

    expect(tasks[0].metadata.dependsOn).toEqual([]);
    expect(tasks[0].metadata.cwd).toBe("./test-app");
    expect(tasks[0].metadata.allowedTools).toEqual(["Bash", "Write", "Read"]);

    expect(tasks[1].metadata.dependsOn).toEqual(["setup-project"]);
    expect(tasks[3].metadata.dependsOn).toEqual(["build-frontend", "build-backend"]);
  });

  it("extracts instruction text (excluding yaml block)", () => {
    const content = readFixture("valid-plan.md");
    const tasks = parseTaskSections(content);
    expect(tasks[0].instruction).toBe(
      "Initialize a new project with basic configuration."
    );
  });

  it("handles single task", () => {
    const content = readFixture("minimal-plan.md");
    const tasks = parseTaskSections(content);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].id).toBe("only-task");
  });

  it("throws on invalid task ID", () => {
    const content = `---\nname: "t"\ngoal: "g"\n---\n\n## INVALID_ID\n\n\`\`\`yaml\ndependsOn: []\ncwd: "."\n\`\`\`\n\nDo stuff.\n`;
    expect(() => parseTaskSections(content)).toThrow("Invalid task ID");
  });

  it("throws on missing yaml block", () => {
    const content = `---\nname: "t"\ngoal: "g"\n---\n\n## my-task\n\nNo yaml block here.\n`;
    expect(() => parseTaskSections(content)).toThrow("missing a yaml metadata block");
  });
});

describe("parseSpec", () => {
  it("parses a complete valid spec", () => {
    const content = readFixture("valid-plan.md");
    const spec = parseSpec(content);
    expect(spec.frontmatter.name).toBe("test-project");
    expect(spec.tasks).toHaveLength(4);
  });

  it("throws on empty task list", () => {
    const content = "---\nname: \"t\"\ngoal: \"g\"\n---\n";
    expect(() => parseSpec(content)).toThrow("at least one task");
  });
});
