# orcha-ai

Orchestrate multiple Claude Code agents from a structured markdown spec. Define tasks, dependencies, and constraints in a single markdown file — orcha parses it into a DAG, runs tasks in parallel where possible, and logs everything.

## Install

```sh
npm install
npm run build
```

## Usage

### Run a spec

```sh
orcha run plan.md
```

Options:

| Flag | Description | Default |
|------|-------------|---------|
| `--dry-run` | Validate and print DAG without running agents | `false` |
| `-c, --concurrency <n>` | Max concurrent agents | `4` |
| `--logs-dir <path>` | Directory for run logs | `./orcha-logs` |

### Create a spec interactively

```sh
orcha init
orcha init -o my-plan.md
```

Walks you through project name, goal, model, and tasks. Outputs a markdown spec you can review and edit before running.

### Development

```sh
npx tsx src/index.ts run plan.md          # run without building
npx tsx src/index.ts run plan.md --dry-run # validate only
npm test                                   # run tests
```

## Spec Format

A spec file is markdown with YAML frontmatter and one H2 section per task.

````markdown
---
name: "my-web-app"
goal: "Build a landing page with contact form"
model: "claude-sonnet-4-20250514"
maxBudgetUsd: 5.00
---

## setup-project

```yaml
dependsOn: []
cwd: "./my-web-app"
allowedTools: ["Bash", "Write", "Read"]
```

Initialize a Vite + React + TypeScript project with Tailwind.

## build-landing-page

```yaml
dependsOn: ["setup-project"]
cwd: "./my-web-app"
allowedTools: ["Read", "Write", "Edit", "Glob"]
```

Build a hero section, features section, and footer.

## build-contact-form

```yaml
dependsOn: ["setup-project"]
cwd: "./my-web-app"
```

Create a contact form with validation.

## integrate-and-test

```yaml
dependsOn: ["build-landing-page", "build-contact-form"]
cwd: "./my-web-app"
allowedTools: ["Read", "Write", "Edit", "Bash", "Glob", "Grep"]
```

Integrate components, run build, fix errors.
````

### Frontmatter

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Project name |
| `goal` | yes | High-level goal passed to every agent as context |
| `model` | no | Default model for all tasks |
| `maxBudgetUsd` | no | Budget cap |

### Task fields

| Field | Required | Description |
|-------|----------|-------------|
| `dependsOn` | yes | Array of task IDs this task waits for |
| `cwd` | no | Working directory for the agent |
| `allowedTools` | no | Tool allowlist (e.g. `["Read", "Write", "Bash"]`) |
| `model` | no | Model override for this task |
| `maxBudgetUsd` | no | Per-task budget cap |

Task IDs must be lowercase alphanumeric with hyphens (e.g. `setup-project`). The prose below the yaml block is the instruction sent to the agent.

## How It Works

```
plan.md
  -> parse markdown (frontmatter + task sections)
  -> validate (ref integrity, cycle detection)
  -> build DAG (reverse edges, roots, topological order)
  -> scheduler emits "taskReady" for root nodes
  -> engine runs agents in parallel (up to maxConcurrency)
     -> each agent gets project context + upstream task summaries
     -> calls @anthropic-ai/claude-code SDK query()
     -> streams messages to per-task log file
  -> on completion: unblock dependents or cascade failure
  -> write summary.json, print report
```

Tasks with no dependency relationship run in parallel. If a task fails, all transitive dependents are cancelled.

## Logs

Each run creates a directory under `./orcha-logs/`:

```
orcha-logs/
  2026-02-20T22-30-00-000Z-my-web-app/
    dag.json                  # parsed DAG structure
    tasks/
      setup-project.log       # NDJSON agent messages
      setup-project.result.json
      build-landing-page.log
      build-landing-page.result.json
      ...
    summary.json              # run stats, costs, durations
```

## Tests

```sh
npm test
```

39 tests covering the parser, validator, DAG builder, scheduler, logger, and agent (mocked SDK).
