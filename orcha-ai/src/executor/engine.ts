import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseSpec } from "../parser/markdown.js";
import { validateSpec } from "../parser/validator.js";
import { buildDag } from "../dag/builder.js";
import { DagScheduler } from "../dag/scheduler.js";
import { RunLogger } from "../logger/run-logger.js";
import { runAgent } from "./agent.js";
import type { Dag, TaskResult } from "../dag/types.js";
import type { SpecFile } from "../parser/schema.js";
import * as consoleLogger from "../logger/console.js";

export interface EngineOptions {
  specPath: string;
  maxConcurrency?: number;
  dryRun?: boolean;
  logsDir?: string;
}

export interface EngineResult {
  spec: SpecFile;
  dag: Dag;
  results: TaskResult[];
}

export async function runEngine(opts: EngineOptions): Promise<EngineResult> {
  const { specPath, maxConcurrency = 4, dryRun = false, logsDir = "./orcha-logs" } = opts;

  // Parse
  const content = readFileSync(resolve(specPath), "utf-8");
  const spec = parseSpec(content);

  // Validate
  const errors = validateSpec(spec);
  if (errors.length > 0) {
    const messages = errors.map((e) => `  - ${e.message}`).join("\n");
    throw new Error(`Spec validation failed:\n${messages}`);
  }

  // Build DAG
  const dag = buildDag(spec);

  if (dryRun) {
    console.log("Dry run - DAG validated successfully.");
    console.log(`Tasks: ${spec.tasks.map((t) => t.id).join(", ")}`);
    console.log(`Topological order: ${dag.topologicalOrder.join(" -> ")}`);
    console.log(`Roots: ${dag.roots.join(", ")}`);
    return { spec, dag, results: [] };
  }

  // Set up logger
  const logger = new RunLogger(logsDir, spec.frontmatter.name);
  logger.writeDag(dag);

  // Set up scheduler
  const scheduler = new DagScheduler(dag, maxConcurrency);
  const results: TaskResult[] = [];
  const summaries = new Map<string, string>();
  const abortController = new AbortController();

  // Graceful shutdown
  const shutdown = () => {
    console.log("\nShutting down gracefully...");
    abortController.abort();
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  return new Promise<EngineResult>((resolvePromise) => {
    scheduler.on("taskReady", async (taskId: string) => {
      const node = dag.nodes.get(taskId)!;
      scheduler.markRunning(taskId);
      consoleLogger.logTaskRunning(taskId);

      // Collect upstream summaries
      const upstreamSummaries = new Map<string, string>();
      for (const depId of node.dependsOn) {
        const depNode = dag.nodes.get(depId);
        if (depNode?.result?.summary) {
          upstreamSummaries.set(depId, depNode.result.summary);
        }
      }

      const result = await runAgent({
        frontmatter: spec.frontmatter,
        task: node.task,
        upstreamSummaries,
        logger,
        abortController,
      });

      results.push(result);
      logger.writeTaskResult(taskId, result);
      consoleLogger.logTaskCompleted(taskId, result);

      if (result.success) {
        summaries.set(taskId, result.summary);
      }

      scheduler.complete(taskId, result);
    });

    scheduler.on("allCompleted", () => {
      process.off("SIGINT", shutdown);
      process.off("SIGTERM", shutdown);

      const summary = logger.writeSummary(results);
      consoleLogger.logSummary(results);
      console.log(`\nLogs written to: ${logger.runDir}`);
      resolvePromise({ spec, dag, results });
    });

    scheduler.start();
  });
}
