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
In a monorepo, put the Skill inside the published package directory, so the package ships it.
Write one Skill for the installed package, unless a second package has distinct users.
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
   To pack a local build, use the repository's package manager. In a pnpm workspace, run `pnpm pack`, because `npm pack` leaves `catalog:` versions.
   Install only the package and the peers the documentation names. A resolution error is a finding.
2. Use the consumer defaults. The package repository's own config and fixtures can turn defaults off.
   In a Nuxt fixture, keep test modules out of `modules/`, because Nuxt registers every module in that folder.
   One fixture page can exercise many examples, so each build tests more claims.
3. Run each example you plan to include. Compare the output with the claim: rendered HTML, return values, type errors, build logs, exit codes, or report files.
   For a framework module, test each mode it supports. Read dev warnings in the dev server log, prerender results in the build output, and runtime results from the built production server.
   If a trap says nothing happens, run it and confirm the silence.
   To fetch pages from a server, run this Skill's [scripts/serve-fixture.mjs](scripts/serve-fixture.mjs), for example `node SKILL_DIR/scripts/serve-fixture.mjs --fetch / -- node .output/server/index.mjs` from the fixture directory.
   It uses a free port, fetches with `Accept: text/html`, and stops only its own process group.
   Without `--fetch`, it holds the server until SIGTERM. Run that with your tool's background option, because `&` and `nohup` die when the shell call ends.
   Never kill by port or with `pkill -f`. Another Agent can own that process, and the pattern can match your own shell.
   In a browser test, set a desktop browser user agent. A package can treat `HeadlessChrome` as a bot.
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
- A deploy section, such as CI cache or Cloudflare, when most traps live there.

Cut:

- Explanations of the domain, the framework, or the language.
- Config options whose name and type explain them. Link the config reference instead. Keep a table when the values are the API, such as priority numbers.
- Internals the consumer cannot act on, such as tree shaking or tag priority.
- Changelog paraphrase, fixed bugs, and release history.
- Generic debug advice, such as "view the page source" or public validators.
- `path:line` citations. The consumer does not read the package source. Put evidence in the report.
- Any second copy of content, including a list that repeats the inline reference links.

Shape:

- Name the package and the tested version in the first paragraph. The frontmatter has no version field.
- Use this order and drop empty sections: setup, automatic behaviour, common tasks, integrations (such as Nuxt Content or i18n), traps, version limits, config, debug.
- Put a config example that is also a common task under common tasks. The config section only lists options.
- Write each code block as a complete module with its imports. The repository ESLint config can lint fenced code.
- If a trap's detail lives in a reference, write one line in traps that links the reference. Keep the detail only in the reference.
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
- If the package ships the Skill: add the Skill directory to `files` in `package.json`. `npm pack --dry-run` runs `prepack`. To list the files without a rebuild, build once, then add `--ignore-scripts`. That flag is npm only; `pnpm pack` rejects it.
- If the README or docs have a `skilld add <package>` tip, replace it with the tip below, in the same place. Add the badge after the other badges.
  Replace `OWNER`, `REPOSITORY`, and `PACKAGE`. If the Repository has more than one `SKILL.md`, add `/SKILL` to both the page path and the badge path. The skilld.dev indexer counts every `SKILL.md` in the tree, test fixtures included. The page shows the run command, so the README does not repeat it.

```html
<a href="https://skilld.dev/gh/OWNER/REPOSITORY">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://skilld.dev/b/OWNER/REPOSITORY?theme=dark">
    <source media="(prefers-color-scheme: light)" srcset="https://skilld.dev/b/OWNER/REPOSITORY?theme=light">
    <img alt="Skill repository on skilld.dev" src="https://skilld.dev/b/OWNER/REPOSITORY?theme=light">
  </picture>
</a>
```

```md
> [!TIP]
> Using an AI agent? Get the PACKAGE Skill on [skilld.dev/gh/OWNER/REPOSITORY](https://skilld.dev/gh/OWNER/REPOSITORY).
```

For a direct run, show the files for review, or open a pull request if the user asks.
Replace an existing Skill only after the user approves it.
A direct run has no Harness checks. Do not claim that it passed them.
