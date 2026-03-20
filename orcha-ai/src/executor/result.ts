import { z } from "zod";

export const TaskCompletionSchema = z.object({
  success: z.boolean(),
  summary: z.string(),
  filesModified: z.array(z.string()).default([]),
});

export type TaskCompletion = z.infer<typeof TaskCompletionSchema>;
