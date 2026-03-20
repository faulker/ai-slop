import { query, type SDKMessage, type SDKResultMessage } from "@anthropic-ai/claude-code";
import type { Frontmatter, TaskDefinition } from "../parser/schema.js";
import type { TaskResult } from "../dag/types.js";
import type { RunLogger } from "../logger/run-logger.js";

export interface AgentOptions {
  frontmatter: Frontmatter;
  task: TaskDefinition;
  upstreamSummaries: Map<string, string>;
  logger: RunLogger;
  abortController?: AbortController;
}

function buildPrompt(opts: AgentOptions): string {
  const { frontmatter, task, upstreamSummaries } = opts;

  let prompt = `# Project: ${frontmatter.name}\n`;
  prompt += `## Goal: ${frontmatter.goal}\n\n`;

  if (upstreamSummaries.size > 0) {
    prompt += `## Completed upstream tasks:\n`;
    for (const [id, summary] of upstreamSummaries) {
      prompt += `- **${id}**: ${summary}\n`;
    }
    prompt += "\n";
  }

  prompt += `## Your task: ${task.id}\n\n`;
  prompt += task.instruction;
  prompt += "\n\nWhen you are done, provide a brief summary of what you accomplished.";

  return prompt;
}

export async function runAgent(opts: AgentOptions): Promise<TaskResult> {
  const { task, frontmatter, logger, abortController } = opts;
  const startTime = Date.now();

  const prompt = buildPrompt(opts);
  const model = task.metadata.model ?? frontmatter.model;

  try {
    const response = query({
      prompt,
      options: {
        cwd: task.metadata.cwd,
        allowedTools: task.metadata.allowedTools,
        model,
        maxTurns: 50,
        permissionMode: "bypassPermissions",
        abortController,
      },
    });

    let resultMessage: SDKResultMessage | undefined;
    let lastAssistantText = "";

    for await (const message of response) {
      logger.appendTaskLog(task.id, {
        timestamp: new Date().toISOString(),
        type: message.type,
        ...(message.type === "assistant"
          ? { content: message.message.content }
          : {}),
      });

      if (message.type === "assistant") {
        const textBlocks = message.message.content.filter(
          (b): b is { type: "text"; text: string } => b.type === "text"
        );
        if (textBlocks.length > 0) {
          lastAssistantText = textBlocks[textBlocks.length - 1].text;
        }
      }

      if (message.type === "result") {
        resultMessage = message as SDKResultMessage;
      }
    }

    const durationMs = Date.now() - startTime;

    if (resultMessage?.subtype === "success") {
      return {
        taskId: task.id,
        success: true,
        summary: resultMessage.result ?? lastAssistantText.slice(0, 500),
        filesModified: [],
        durationMs,
        costUsd: resultMessage.total_cost_usd ?? 0,
      };
    }

    return {
      taskId: task.id,
      success: false,
      summary: lastAssistantText.slice(0, 500),
      filesModified: [],
      durationMs,
      costUsd: resultMessage?.total_cost_usd ?? 0,
      error: resultMessage?.subtype === "error_max_turns"
        ? "Max turns exceeded"
        : "Execution error",
    };
  } catch (err) {
    const durationMs = Date.now() - startTime;
    return {
      taskId: task.id,
      success: false,
      summary: "",
      filesModified: [],
      durationMs,
      costUsd: 0,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}
