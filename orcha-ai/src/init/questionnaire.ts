import inquirer from "inquirer";

interface TaskInput {
  id: string;
  description: string;
  dependsOn: string;
  cwd: string;
}

export async function runQuestionnaire(): Promise<string> {
  const project = await inquirer.prompt([
    {
      type: "input",
      name: "name",
      message: "Project name:",
      validate: (v: string) => v.trim().length > 0 || "Required",
    },
    {
      type: "input",
      name: "goal",
      message: "Project goal:",
      validate: (v: string) => v.trim().length > 0 || "Required",
    },
    {
      type: "input",
      name: "model",
      message: "Model (leave blank for default):",
      default: "",
    },
    {
      type: "input",
      name: "maxBudgetUsd",
      message: "Max budget in USD (leave blank for no limit):",
      default: "",
    },
  ]);

  const tasks: TaskInput[] = [];
  let addMore = true;

  while (addMore) {
    const taskAnswers = await inquirer.prompt([
      {
        type: "input",
        name: "id",
        message: "Task ID (lowercase, hyphens allowed):",
        validate: (v: string) =>
          /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(v) || "Must be lowercase alphanumeric with hyphens",
      },
      {
        type: "input",
        name: "description",
        message: "Task instruction:",
        validate: (v: string) => v.trim().length > 0 || "Required",
      },
      {
        type: "input",
        name: "dependsOn",
        message: "Dependencies (comma-separated task IDs, or empty):",
        default: "",
      },
      {
        type: "input",
        name: "cwd",
        message: "Working directory (relative path):",
        default: ".",
      },
    ]);

    tasks.push(taskAnswers);

    const { more } = await inquirer.prompt([
      {
        type: "confirm",
        name: "more",
        message: "Add another task?",
        default: false,
      },
    ]);
    addMore = more;
  }

  return generateMarkdown(project, tasks);
}

function generateMarkdown(
  project: { name: string; goal: string; model: string; maxBudgetUsd: string },
  tasks: TaskInput[]
): string {
  let md = "---\n";
  md += `name: "${project.name}"\n`;
  md += `goal: "${project.goal}"\n`;
  if (project.model) {
    md += `model: "${project.model}"\n`;
  }
  if (project.maxBudgetUsd) {
    md += `maxBudgetUsd: ${parseFloat(project.maxBudgetUsd)}\n`;
  }
  md += "---\n\n";

  for (const task of tasks) {
    md += `## ${task.id}\n\n`;
    const deps = task.dependsOn
      ? task.dependsOn.split(",").map((d) => `"${d.trim()}"`).join(", ")
      : "";
    md += "```yaml\n";
    md += `dependsOn: [${deps}]\n`;
    md += `cwd: "${task.cwd}"\n`;
    md += "```\n\n";
    md += `${task.description}\n\n`;
  }

  return md;
}
