---
name: generate-package-skill
description: Generate or update an Agent Skill for an npm or local package using its public API and current documentation.
---

# Generate a package Skill

Create a focused Skill that helps an Agent use one package correctly.
This Skill is for maintainers who author a draft Skill they own.

## Inputs

Ask for a package name, package directory, or prepared package source.
Ask for the destination only when the request does not provide one.

## Research

1. Record the exact installed or prepared package version.
2. Read the package manifest and every exported entry point.
3. Read public type entry points and their source definitions.
4. Read current official documentation and runnable examples.
5. Check release notes and migration guides for the exact version.
6. Prefer public exports over internal files.
7. Record advice only when the prepared source or official documentation proves it.

For an npm package, use the installed or prepared package source first.
Use current official documentation when the prepared source lacks required details.
For each version-specific rule, cite the source path or official documentation URL.
Use `path:line` citations for prepared source when line numbers add value.

## Output

Write one directory whose name matches the Skill name.
The directory must contain `SKILL.md`.
Put detailed or conditional material in `references/`.
Put reusable commands or code in `scripts/` when execution adds value.

The `SKILL.md` frontmatter must contain only:

```yaml
---
name: package-name
description: Clear trigger conditions and the result this Skill provides.
---
```

Use lowercase letters, numbers, and single hyphens in the name.
Keep the name at 64 characters or fewer.
Keep the description at 1024 characters or fewer.

## Quality checks

- Keep instructions specific to the package.
- Use current APIs and package vocabulary.
- Include small examples for common tasks.
- State environment or version limits.
- Cite public APIs, version limits, and migration advice.
- Link each reference from `SKILL.md`.
- Remove generated filler and repeated prose.
- Do not copy large documentation sections.
- Do not include credentials, caches, build output, or dependency directories.

For a direct run, show the generated files for user review.
Replace an existing Skill only after the user approves the files.
Do not claim that the Skill passed Harness checks.
