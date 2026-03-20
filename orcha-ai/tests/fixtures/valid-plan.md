---
name: "test-project"
goal: "Build a test application"
model: "claude-sonnet-4-20250514"
maxBudgetUsd: 5.00
---

## setup-project

```yaml
dependsOn: []
cwd: "./test-app"
allowedTools: ["Bash", "Write", "Read"]
```

Initialize a new project with basic configuration.

## build-frontend

```yaml
dependsOn: ["setup-project"]
cwd: "./test-app"
allowedTools: ["Read", "Write", "Edit"]
```

Build the frontend components.

## build-backend

```yaml
dependsOn: ["setup-project"]
cwd: "./test-app"
allowedTools: ["Read", "Write", "Edit"]
```

Build the backend API endpoints.

## integrate

```yaml
dependsOn: ["build-frontend", "build-backend"]
cwd: "./test-app"
allowedTools: ["Read", "Write", "Edit", "Bash"]
```

Integrate frontend and backend, run tests.
