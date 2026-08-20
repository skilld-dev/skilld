---
name: generate-project-skill
description: Generate or update an Agent Skill from the observed workflows, boundaries, and conventions of one software project.
---

# Generate a project Skill

Create a compact Skill from the project itself.

## Inputs

Use the project directory or prepared project source.
Ask for the destination only when the request does not provide one.

## Inspect

1. Read project instructions and decision records first.
2. Read manifests, exported entry points, and build configuration.
3. Find the test, lint, typecheck, build, and release commands.
4. Trace the main public workflows through their real entry points.
5. Note generated directories and files an Agent must not edit.
6. Check recent changes when they explain current conventions.

Ignore dependency directories, version-control data, generated output, caches, and credentials.
Do not follow symbolic links while collecting project files.

## Output

Write one directory whose name matches the Skill name.
The directory must contain `SKILL.md`.
Use `references/` for architecture details and command guides.
Use `scripts/` only for reusable automation.

The `SKILL.md` frontmatter must contain only `name` and `description`.
Use lowercase letters, numbers, and single hyphens in the name.
Keep the name at 64 characters or fewer.

## Quality checks

- Describe when the Skill applies.
- Use project terms exactly.
- Point to source files instead of copying them.
- Include verified commands and expected outcomes.
- Separate rules from optional guidance.
- Remove stale, inferred, or duplicated instructions.
- Link each reference from `SKILL.md`.

For a direct run, show the generated files for user review.
Do not replace an existing Skill until the user approves it.
