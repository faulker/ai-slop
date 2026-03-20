import { Command } from "commander";
import { runCommand } from "./commands/run.js";
import { initCommand } from "./commands/init.js";

const program = new Command();

program
  .name("orcha")
  .description("Orchestrate multiple Claude Code agents from a markdown spec")
  .version("0.1.0");

program
  .command("run")
  .description("Run a spec file")
  .argument("<file>", "Path to the markdown spec file")
  .option("--dry-run", "Validate and show DAG without running agents")
  .option("-c, --concurrency <number>", "Max concurrent agents", "4")
  .option("--logs-dir <path>", "Directory for run logs", "./orcha-logs")
  .action(runCommand);

program
  .command("init")
  .description("Interactively create a new spec file")
  .option("-o, --output <path>", "Output file path", "orcha-spec.md")
  .action(initCommand);

program.parse();
