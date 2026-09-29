---
name: generate-package-skill
description: Generate or update an Agent Skill that teaches consumers one npm or local package the user maintains, including framework modules and wrappers. Tests each example against the installed version. Use when a maintainer asks for a package Skill, a SKILL.md for their library, or a Skill update after a release.
---

# Generate a package Skill

Write a short Skill that stops an Agent from misusing one package version.
The reader already knows the language, the framework, and the domain.
Write only what it would get wrong without the Skill.

## Inputs

Use the package name, package directory, or prepared package source.
Ask for the destination only when the request does not give one.
`assets/` holds the Harness request. A direct run ignores it.

## 1. Research

1. Record the exact package version.
2. Read the manifest, every exported entry point, and the public types.
3. If the package is a framework module, read what it registers at setup: auto-imports, components, the config key and its defaults, hooks, and server routes. For a Nuxt module, `defineNuxtModule` in the module entry registers them.
4. If the package re-exports or wraps another package, or its API depends on a peer dependency, record the version the consumer gets. Read that version's types or tagged source. Never read a default branch.
5. Read the current official documentation and examples.
6. Read release notes only for breaking changes and removed APIs.

If a Skill already exists, treat its claims as input to test. Do not copy its layout.

## 2. Test the examples

Documentation can be wrong. Observed behaviour wins.

1. Create a minimal consumer fixture outside the package source. Install the recorded version, or link the local build.
2. Use the consumer defaults. The package repository's own config and fixtures can turn defaults off.
3. Run each example you plan to include. Compare the output with the claim: rendered HTML, return values, or type errors.
4. If the documentation and the behaviour disagree, write the behaviour and add the mismatch to the report.
5. If you cannot run an example, keep it only when the types prove it. List it as untested in the report.

Do not fix the package or its documentation in the Skill change.

## 3. Write

Include:

- Setup steps that differ from the framework default, such as a required config key.
- What the package does automatically, and how to turn it off.
- Traps: silent failures, plausible but wrong calls, documentation mismatches, and version limits.
- One small example per common task, when the correct call is not obvious from the types.
- Breaking changes an Agent trained on an older version would repeat. Show the old call and the new call.

Cut:

- Explanations of the domain, the framework, or the language.
- Config options whose name and type explain them. Link the config reference instead.
- Internals the consumer cannot act on, such as tree shaking or tag priority.
- Changelog paraphrase, fixed bugs, and release history.
- Generic debug advice, such as "view the page source" or public validators.
- `path:line` citations. The consumer does not read the package source. Put evidence in the report.
- Any second copy of content, including a list that repeats the inline reference links.

Shape:

- Name the package and the tested version in the first paragraph. The frontmatter has no version field.
- Use this order and drop empty sections: setup, automatic behaviour, common tasks, traps, version limits, config, debug.
- Aim for 150 lines or fewer in `SKILL.md`. Never exceed 500.
- Keep one file by default. Move a topic to `references/<topic>.md` only when it passes about 40 lines and applies to under a third of tasks, such as one integration.
- Link each reference from `SKILL.md`. Keep references one level deep. Start a reference over 100 lines with a contents list.
- Write at most eight reference files. Add `scripts/` only when running code beats reading it.
- Never include credentials, caches, build output, or dependency directories.

The frontmatter contains only `name` and `description`.
The name uses lowercase letters, numbers, and single hyphens, at most 64 characters, and matches the directory.
The description is at most 1024 characters in the third person. It says what the Skill does, then when to use it, with the words a user types: the package name, main exports, the config key, and error symptoms.

- Good: `Add and debug Schema.org JSON-LD in Nuxt with nuxt-schema-org. Use when a task mentions structured data, rich results, useSchemaOrg, defineArticle, or the schemaOrg config key.`
- Bad: `Helps with nuxt-schema-org.`

## 4. Check and report

Before you finish, confirm:

- Each example ran against the recorded version, or the report lists it as untested.
- Each sentence tells the reader something it would otherwise get wrong.
- Each reference is linked from `SKILL.md`, and no file is unlinked. Delete stale files from an earlier Skill.
- The frontmatter follows the rules above.

Report to the user:

- The files and the tested version.
- Each documentation and behaviour mismatch: the example, the documented result, and the observed result. These are package bugs for the maintainer.
- Each untested example.
- The source path or documentation URL behind each version-specific rule.
- If the package ships the Skill: add the Skill directory to `files` in `package.json`, and replace `skilld add <package>` tips in the README and docs with `skilld run OWNER/REPOSITORY/SKILL`. `npm pack --dry-run` runs `prepack`. To list the files without a rebuild, build once, then add `--ignore-scripts`.

For a direct run, show the files for review, or open a pull request if the user asks.
Replace an existing Skill only after the user approves it.
A direct run has no Harness checks. Do not claim that it passed them.
