---
name: generate-package-skill
description: Generate or update an Agent Skill that teaches consumers one npm or local package the user maintains, including framework modules and wrappers. Tests each example against the installed version. Use when a maintainer asks for a package Skill, a SKILL.md for their library, or a Skill update after a release.
---

# Generate a package Skill

Write a short Skill that stops an Agent from misusing one package version.
The reader knows the language, the framework, and the domain. Write only what it would get wrong.

## Inputs

Take a package name, directory, or prepared source. Ask for the destination only when it is missing.
In a monorepo, put the Skill in the published package directory.
Write one Skill per installed package, unless a second package has distinct users.
`assets/` holds the Harness request. A direct run ignores it.

## 1. Research

1. Record the exact package version.
2. Read the manifest, every exported entry point, and the public types.
3. For a framework module, read what setup registers: auto-imports, components, the config key and defaults, hooks, server routes.
4. If the package wraps, re-exports, or peers on another package, read the types or tagged source of the version the consumer gets, never a default branch.
5. Read the current official docs and examples, and release notes only for breaking changes and removed APIs.

Test an existing Skill's claims; do not copy its layout.

## 2. Test the examples

Observed behaviour beats documentation.

1. Create a minimal consumer fixture outside the package source. Install the recorded version, or link the local build.
   Before packing, build the package and its native code (NAPI, WASM); `prepack` can need them.
   Pack with the repository's package manager: `npm pack` leaves pnpm `catalog:` versions.
   Install only the package and documented peers. A resolution error is a finding.
2. Use consumer defaults. The package repository's config and fixtures can turn them off.
   In Nuxt, keep test modules out of `modules/`; Nuxt registers every module there.
   One fixture page can exercise many examples.
3. Run each example you include. Compare output with the claim: HTML, return values, type errors, build logs, exit codes, report files.
   Test each mode: framework modes, export conditions (`node`, `workerd`, `edge-light`, `browser`, CDN or IIFE build), and each CLI binary with a config file and with flags.
   Wrap every run in `timeout 120`.
   Grep the package for agent and CI detection, such as `CLAUDECODE` or `CI`; unset each variable it reads with `env -u VAR`.
   Read dev warnings in the dev log, prerender results in build output, and runtime results from the production server.
   If a trap says nothing happens, run it and confirm the silence.
   To fetch from a server, run [scripts/serve-fixture.mjs](scripts/serve-fixture.mjs) in the fixture: `node SKILL_DIR/scripts/serve-fixture.mjs --fetch / -- node .output/server/index.mjs`.
   For a binary, use `--fetch-raw PATH --out DIR`. `DIR/responses.json` lists each status and content type.
   Without `--fetch`, it holds the server until SIGTERM. Background it with your tool's option; `&` and `nohup` die with the shell call.
   Never kill by port or with `pkill -f`: another Agent can own that process.
   In a browser, set a desktop user agent; a package can treat `HeadlessChrome` as a bot.
4. If documentation and behaviour disagree, write the behaviour and report the mismatch.
5. Keep an unrunnable example only when the types prove it; report it untested.

Do not fix the package or its docs in the Skill change.

## 3. Write

Include:

- Setup that differs from the framework default, such as a required config key.
- What the package does automatically, and how to turn it off.
- Traps: silent failures, plausible wrong calls, documentation mismatches, version limits.
- One small example per common task the types do not make obvious.
- Breaking changes an Agent trained on an older version would repeat: old call, new call.
- A deploy section, such as CI cache or Cloudflare, when most traps live there.

Cut:

- Self-explanatory config options; link the config reference. Keep a table when values are the API, such as priorities.
- Internals the consumer cannot act on, such as tree shaking.
- Changelog paraphrase, fixed bugs, release history.
- Generic debug advice, such as "view source" or public validators.
- `path:line` citations. Put evidence in the report.
- Any second copy of content, such as a list repeating inline reference links.

Shape:

- Name the package and tested version in the first paragraph, not the frontmatter.
- Order, dropping empty sections: setup, automatic behaviour, common tasks, integrations, traps, version limits, config, debug.
- A config example that is a common task goes there; the config section only lists options.
- Write each code block as a complete module with imports. ESLint can lint fenced code.
- A trap detailed in a reference gets one linking line in traps.
- Aim for 150 lines and about 2,000 tokens in `SKILL.md`. Never exceed 500 lines.
- Keep one file. Move a topic to `references/<topic>.md` only past about 40 lines and when under a third of tasks need it.
- Link each reference from `SKILL.md`, one level deep. A reference over 100 lines starts with contents.
- Write at most eight reference files. Add `scripts/` only when running code beats reading it.
- Never include credentials, caches, build output, or dependency directories.

The frontmatter contains only `name` and `description`.
The name uses lowercase letters, numbers, and single hyphens, at most 64 characters, and matches the directory.
For a scoped package, drop the `@` and replace `/` with a hyphen: `@nuxtjs/seo` becomes `nuxtjs-seo`.
The description, at most 1024 characters in third person, says what the Skill does, then when to use it, in the words a user types: package name, main exports, config key, error symptoms.
Good: `Add and debug Schema.org JSON-LD in Nuxt with nuxt-schema-org. Use when a task mentions structured data, rich results, useSchemaOrg, defineArticle, or the schemaOrg config key.`

## 4. Check and report

Before finishing, confirm:

- Each example ran against the recorded version, or is reported untested.
- Each reference is linked. Delete stale files from an earlier Skill.
- The frontmatter follows the rules above.

Report to the user:

- The files and the tested version.
- Each untested example, and each mismatch: example, documented result, observed result. These are package bugs.
- The source path or documentation URL behind each version-specific rule.
- If the package ships the Skill, add its directory to `files` in `package.json`. After one build, list packed files with `npm pack --dry-run --ignore-scripts`; `pnpm pack` rejects that flag.
- Replace any README or docs `skilld add <package>` tip in place with the tip below. Add the badge after the others.
  Replace `OWNER`, `REPOSITORY`, and `PACKAGE`. If the Repository has several Skills, add `/SKILL` to the page and badge paths. The skilld.dev indexer skips `SKILL.md` under test and fixture folders. The README omits the run command; the page shows it.

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

For a direct run, show the files for review, or open a pull request if asked.
Replace an existing Skill only with user approval.
A direct run has no Harness checks; never claim it passed them.
