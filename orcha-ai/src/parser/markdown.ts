import { parse as parseYaml } from "yaml";
import {
  FrontmatterSchema,
  TaskMetadataSchema,
  type Frontmatter,
  type TaskDefinition,
  type SpecFile,
  taskIdPattern,
} from "./schema.js";

const FRONTMATTER_RE = /^---\n([\s\S]*?)\n---/;
const TASK_SECTION_RE = /^## +(\S+)/;
const YAML_BLOCK_RE = /```ya?ml\n([\s\S]*?)```/;

export function parseFrontmatter(content: string): Frontmatter {
  const match = content.match(FRONTMATTER_RE);
  if (!match) {
    throw new Error("Missing YAML frontmatter (expected --- delimiters)");
  }
  const raw = parseYaml(match[1]);
  return FrontmatterSchema.parse(raw);
}

export function parseTaskSections(content: string): TaskDefinition[] {
  const body = content.replace(FRONTMATTER_RE, "").trim();
  const lines = body.split("\n");
  const sections: { id: string; lines: string[] }[] = [];

  for (const line of lines) {
    const heading = line.match(TASK_SECTION_RE);
    if (heading) {
      sections.push({ id: heading[1], lines: [] });
    } else if (sections.length > 0) {
      sections[sections.length - 1].lines.push(line);
    }
  }

  return sections.map((section) => {
    if (!taskIdPattern.test(section.id)) {
      throw new Error(
        `Invalid task ID "${section.id}": must be lowercase alphanumeric with hyphens`
      );
    }

    const sectionContent = section.lines.join("\n");
    const yamlMatch = sectionContent.match(YAML_BLOCK_RE);
    if (!yamlMatch) {
      throw new Error(`Task "${section.id}" is missing a yaml metadata block`);
    }

    const rawMeta = parseYaml(yamlMatch[1]);
    const metadata = TaskMetadataSchema.parse(rawMeta);

    const instruction = sectionContent
      .replace(YAML_BLOCK_RE, "")
      .trim();

    if (!instruction) {
      throw new Error(`Task "${section.id}" has no instruction text`);
    }

    return { id: section.id, metadata, instruction };
  });
}

export function parseSpec(content: string): SpecFile {
  const frontmatter = parseFrontmatter(content);
  const tasks = parseTaskSections(content);

  if (tasks.length === 0) {
    throw new Error("Spec must contain at least one task");
  }

  return { frontmatter, tasks };
}
