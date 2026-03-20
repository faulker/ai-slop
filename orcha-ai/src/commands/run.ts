import { runEngine } from "../executor/engine.js";

export interface RunCommandOptions {
  dryRun?: boolean;
  concurrency?: string;
  logsDir?: string;
}

export async function runCommand(specPath: string, opts: RunCommandOptions): Promise<void> {
  try {
    await runEngine({
      specPath,
      maxConcurrency: opts.concurrency ? parseInt(opts.concurrency, 10) : 4,
      dryRun: opts.dryRun ?? false,
      logsDir: opts.logsDir ?? "./orcha-logs",
    });
  } catch (err) {
    console.error(err instanceof Error ? err.message : String(err));
    process.exit(1);
  }
}
