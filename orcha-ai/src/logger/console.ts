import type { TaskResult } from "../dag/types.js";

const COLORS = {
  reset: "\x1b[0m",
  green: "\x1b[32m",
  red: "\x1b[31m",
  yellow: "\x1b[33m",
  cyan: "\x1b[36m",
  dim: "\x1b[2m",
  bold: "\x1b[1m",
};

export function logTaskReady(taskId: string): void {
  console.log(`${COLORS.cyan}[READY]${COLORS.reset} ${taskId}`);
}

export function logTaskRunning(taskId: string): void {
  console.log(`${COLORS.yellow}[RUNNING]${COLORS.reset} ${taskId}`);
}

export function logTaskCompleted(taskId: string, result: TaskResult): void {
  if (result.success) {
    console.log(
      `${COLORS.green}[DONE]${COLORS.reset} ${taskId} ${COLORS.dim}(${formatDuration(result.durationMs)}, $${result.costUsd.toFixed(4)})${COLORS.reset}`
    );
  } else {
    console.log(
      `${COLORS.red}[FAILED]${COLORS.reset} ${taskId}: ${result.error ?? "unknown error"}`
    );
  }
}

export function logTaskCancelled(taskId: string): void {
  console.log(`${COLORS.dim}[CANCELLED]${COLORS.reset} ${taskId}`);
}

export function logSummary(results: TaskResult[]): void {
  const total = results.length;
  const completed = results.filter((r) => r.success).length;
  const failed = results.filter((r) => !r.success).length;
  const totalCost = results.reduce((sum, r) => sum + r.costUsd, 0);
  const totalDuration = results.reduce((sum, r) => sum + r.durationMs, 0);

  console.log("\n" + "=".repeat(50));
  console.log(`${COLORS.bold}Run Summary${COLORS.reset}`);
  console.log("=".repeat(50));
  console.log(`  Tasks: ${completed}/${total} completed, ${failed} failed`);
  console.log(`  Cost:  $${totalCost.toFixed(4)}`);
  console.log(`  Time:  ${formatDuration(totalDuration)}`);
  console.log("=".repeat(50));
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  return `${minutes}m${remainingSeconds}s`;
}
