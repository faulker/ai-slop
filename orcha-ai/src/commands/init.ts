import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { runQuestionnaire } from "../init/questionnaire.js";

export interface InitCommandOptions {
  output?: string;
}

export async function initCommand(opts: InitCommandOptions): Promise<void> {
  try {
    const markdown = await runQuestionnaire();
    const outputPath = resolve(opts.output ?? "orcha-spec.md");
    writeFileSync(outputPath, markdown);
    console.log(`Spec written to ${outputPath}`);
    console.log(`Run it with: orcha run ${outputPath}`);
  } catch (err) {
    console.error(err instanceof Error ? err.message : String(err));
    process.exit(1);
  }
}
