---
name: "missing-dep"
goal: "This should fail validation"
---

## task-a

```yaml
dependsOn: []
cwd: "."
```

Task A is a root task.

## task-b

```yaml
dependsOn: ["task-a", "nonexistent-task"]
cwd: "."
```

Task B references a nonexistent dependency.
