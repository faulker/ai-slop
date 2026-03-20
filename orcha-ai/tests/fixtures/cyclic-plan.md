---
name: "cyclic-project"
goal: "This should fail validation"
---

## task-a

```yaml
dependsOn: ["task-b"]
cwd: "."
```

Task A depends on B.

## task-b

```yaml
dependsOn: ["task-c"]
cwd: "."
```

Task B depends on C.

## task-c

```yaml
dependsOn: ["task-a"]
cwd: "."
```

Task C depends on A (cycle!).
