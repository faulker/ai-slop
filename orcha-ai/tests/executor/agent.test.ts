import { describe, it, expect, vi, beforeEach } from "vitest";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { RunLogger } from "../../src/logger/run-logger.js";
import type { Frontmatter, TaskDefinition } from "../../src/parser/schema.js";

// Mock the claude-code SDK
vi.mock("@anthropic-ai/claude-code", () => ({
  query: vi.fn(),
}));

import { query } from "@anthropic-ai/claude-code";
import { runAgent } from "../../src/executor/agent.js";

const mockedQuery = vi.mocked(query);

const testFrontmatter: Frontmatter = {
  name: "test-project",
  goal: "Build something",
  model: "claude-sonnet-4-20250514",
};

const testTask: TaskDefinition = {
  id: "test-task",
  metadata: {
    dependsOn: [],
    cwd: ".",
    allowedTools: ["Read", "Write"],
  },
  instruction: "Do the thing",
};

describe("runAgent", () => {
  let logger: RunLogger;
  const cleanupDirs: string[] = [];

  beforeEach(() => {
    vi.clearAllMocks();
    logger = new RunLogger(join(tmpdir(), "orcha-agent-test"), "test");
    cleanupDirs.push(logger.runDir);
  });

  it("returns success result on successful completion", async () => {
    // Mock the async generator
    async function* mockGenerator() {
      yield {
        type: "assistant" as const,
        uuid: "00000000-0000-0000-0000-000000000000",
        session_id: "test",
        message: {
          content: [{ type: "text" as const, text: "I completed the task successfully." }],
        },
        parent_tool_use_id: null,
      };
      yield {
        type: "result" as const,
        uuid: "00000000-0000-0000-0000-000000000001",
        session_id: "test",
        subtype: "success" as const,
        result: "Task completed",
        duration_ms: 1000,
        duration_api_ms: 800,
        is_error: false,
        num_turns: 3,
        total_cost_usd: 0.05,
        usage: {
          input_tokens: 100,
          output_tokens: 200,
          cache_read_input_tokens: 0,
          cache_creation_input_tokens: 0,
        },
        modelUsage: {},
        permission_denials: [],
      };
    }

    mockedQuery.mockReturnValue(mockGenerator() as any);

    const result = await runAgent({
      frontmatter: testFrontmatter,
      task: testTask,
      upstreamSummaries: new Map(),
      logger,
    });

    expect(result.success).toBe(true);
    expect(result.taskId).toBe("test-task");
    expect(result.costUsd).toBe(0.05);
    expect(result.summary).toBe("Task completed");
  });

  it("returns failure result on error", async () => {
    async function* mockGenerator() {
      yield {
        type: "result" as const,
        uuid: "00000000-0000-0000-0000-000000000000",
        session_id: "test",
        subtype: "error_during_execution" as const,
        duration_ms: 500,
        duration_api_ms: 400,
        is_error: true,
        num_turns: 1,
        total_cost_usd: 0.01,
        usage: {
          input_tokens: 50,
          output_tokens: 10,
          cache_read_input_tokens: 0,
          cache_creation_input_tokens: 0,
        },
        modelUsage: {},
        permission_denials: [],
      };
    }

    mockedQuery.mockReturnValue(mockGenerator() as any);

    const result = await runAgent({
      frontmatter: testFrontmatter,
      task: testTask,
      upstreamSummaries: new Map(),
      logger,
    });

    expect(result.success).toBe(false);
    expect(result.error).toBe("Execution error");
  });

  it("returns failure result on thrown error", async () => {
    mockedQuery.mockImplementation(() => {
      throw new Error("SDK crash");
    });

    const result = await runAgent({
      frontmatter: testFrontmatter,
      task: testTask,
      upstreamSummaries: new Map(),
      logger,
    });

    expect(result.success).toBe(false);
    expect(result.error).toBe("SDK crash");
  });

  it("passes correct options to query", async () => {
    async function* mockGenerator() {
      yield {
        type: "result" as const,
        uuid: "00000000-0000-0000-0000-000000000000",
        session_id: "test",
        subtype: "success" as const,
        result: "done",
        duration_ms: 100,
        duration_api_ms: 80,
        is_error: false,
        num_turns: 1,
        total_cost_usd: 0.01,
        usage: {
          input_tokens: 10,
          output_tokens: 10,
          cache_read_input_tokens: 0,
          cache_creation_input_tokens: 0,
        },
        modelUsage: {},
        permission_denials: [],
      };
    }

    mockedQuery.mockReturnValue(mockGenerator() as any);

    await runAgent({
      frontmatter: testFrontmatter,
      task: testTask,
      upstreamSummaries: new Map([["dep-task", "Completed setup"]]),
      logger,
    });

    expect(mockedQuery).toHaveBeenCalledOnce();
    const callArgs = mockedQuery.mock.calls[0][0];
    expect(callArgs.prompt).toContain("test-project");
    expect(callArgs.prompt).toContain("Do the thing");
    expect(callArgs.prompt).toContain("dep-task");
    expect(callArgs.prompt).toContain("Completed setup");
    expect(callArgs.options?.cwd).toBe(".");
    expect(callArgs.options?.allowedTools).toEqual(["Read", "Write"]);
    expect(callArgs.options?.model).toBe("claude-sonnet-4-20250514");
  });

  it("uses task-level model override", async () => {
    async function* mockGenerator() {
      yield {
        type: "result" as const,
        uuid: "00000000-0000-0000-0000-000000000000",
        session_id: "test",
        subtype: "success" as const,
        result: "done",
        duration_ms: 100,
        duration_api_ms: 80,
        is_error: false,
        num_turns: 1,
        total_cost_usd: 0.01,
        usage: {
          input_tokens: 10,
          output_tokens: 10,
          cache_read_input_tokens: 0,
          cache_creation_input_tokens: 0,
        },
        modelUsage: {},
        permission_denials: [],
      };
    }

    mockedQuery.mockReturnValue(mockGenerator() as any);

    const taskWithModel: TaskDefinition = {
      ...testTask,
      metadata: { ...testTask.metadata, model: "claude-opus-4-20250514" },
    };

    await runAgent({
      frontmatter: testFrontmatter,
      task: taskWithModel,
      upstreamSummaries: new Map(),
      logger,
    });

    const callArgs = mockedQuery.mock.calls[0][0];
    expect(callArgs.options?.model).toBe("claude-opus-4-20250514");
  });
});
