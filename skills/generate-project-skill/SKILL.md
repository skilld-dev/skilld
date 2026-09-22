---
name: generate-project-skill
description: Generate or update an Agent Skill from the observed workflows, boundaries, and conventions of one software project.
---

# Generate a project Skill

Create a compact, searchable Skill from the project itself.
This Skill is for maintainers who author a draft Skill they own.

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

## Project navigation

The Skill must let an Agent find any part of the project without a prior index.

1. Name the project from its manifest. Use the directory name when no manifest names it.
2. List the entry points the manifest declares, such as `main`, `module`, `types`, `exports`, and `bin`.
3. List the source and documentation directories an Agent reads most.
4. Use project-relative paths for every file pointer.
5. Never add a generated directory prefix to a project-relative path.
6. Give search commands the Agent can repeat, scoped to those directories.
7. Prefer `rg` for search. Name the directories to skip in the command.
8. Tell the Agent when project changes require a new Skill run.

Write the search commands so their results point at real project files.

Skip these paths while collecting project files:

- version-control data, such as `.git`
- dependency directories, such as `node_modules` and `vendor`
- generated output, such as `dist`, `build`, `target`, `.output`, `.nuxt`, and `.next`
- caches and reports, such as `coverage`
- credential files, such as `.env` and any private key

Skip any file larger than 512 KB.
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
- Run project commands only when they add useful evidence.
- Record the observed command and outcome.
- Include repeatable search commands for project source and documentation.
- Keep file pointers rooted at the real project directory.
- Separate rules from optional guidance.
- Remove stale, inferred, or duplicated instructions.
- Link each reference from `SKILL.md`.

For a direct run, show the generated files for user review.
Do not replace an existing Skill until the user approves it.
Do not claim that the Skill passed Harness checks.
