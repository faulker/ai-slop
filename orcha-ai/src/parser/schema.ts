import { z } from "zod";

export const taskIdPattern = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

export const FrontmatterSchema = z.object({
  name: z.string().min(1),
  goal: z.string().min(1),
  model: z.string().optional(),
  maxBudgetUsd: z.number().positive().optional(),
});

export type Frontmatter = z.infer<typeof FrontmatterSchema>;

export const TaskMetadataSchema = z.object({
  dependsOn: z.array(z.string()).default([]),
  cwd: z.string().optional(),
  allowedTools: z.array(z.string()).optional(),
  model: z.string().optional(),
  maxBudgetUsd: z.number().positive().optional(),
});

export type TaskMetadata = z.infer<typeof TaskMetadataSchema>;

export const TaskDefinitionSchema = z.object({
  id: z.string().regex(taskIdPattern, "Task ID must be lowercase alphanumeric with hyphens"),
  metadata: TaskMetadataSchema,
  instruction: z.string().min(1, "Task must have an instruction"),
});

export type TaskDefinition = z.infer<typeof TaskDefinitionSchema>;

export const SpecFileSchema = z.object({
  frontmatter: FrontmatterSchema,
  tasks: z.array(TaskDefinitionSchema).min(1, "Spec must contain at least one task"),
});

export type SpecFile = z.infer<typeof SpecFileSchema>;
