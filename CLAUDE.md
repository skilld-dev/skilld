# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
pnpm build          # Build with obuild
pnpm dev:prepare    # Stub for development (obuild --stub)
pnpm typecheck      # TypeScript check (tsc --noEmit)
pnpm lint           # ESLint (@antfu/eslint-config)
pnpm lint:fix       # ESLint with auto-fix
pnpm test           # Run vitest in watch mode
pnpm test:run       # Run vitest once (CI-style)
pnpm test -- test/unit/cache.test.ts     # Single test file
pnpm test -- --project unit              # Unit tests only
pnpm test -- --project e2e               # E2E tests only
```

### CLI Commands

```bash
skilld                          # Interactive menu
skilld add npm:vue npm:nuxt     # Install package skills from registry
skilld add gh:owner/repo        # Install git skills from GitHub
skilld add @curator             # Install all skills from a curator (coming soon)
skilld add @curator/collection  # Install a specific collection (coming soon)
skilld add vue                  # Bare names deprecated, resolves as npm:vue with warning
skilld update                   # Update all outdated skills
skilld update vue               # Update specific package
skilld remove                   # Remove installed skills
skilld list                     # List installed skills (one per line)
skilld list --json              # List as JSON
skilld info                     # Show config, agents, features, per-package detail
skilld config                   # Change settings
skilld install                  # Restore references from lockfile
skilld prepare                  # Hook for package.json "prepare" (restore refs, sync shipped, report outdated)
skilld uninstall                # Remove skilld data
skilld search "query"           # Search indexed docs
skilld search "query" -p nuxt   # Search filtered by package
skilld cache --clean            # Clean expired LLM cache entries
skilld cache --stats            # Show cache disk usage breakdown
```

### Author Commands (skill creation and publishing)

```bash
skilld author package               # Generate a package skill from docs (monorepo-aware)
skilld author package -m haiku      # Author with specific LLM model
skilld author package -o ./custom/  # Author to custom output directory
skilld author publish               # Publish skill list to skilld.dev
skilld author eject vue             # Eject skill (portable, no symlinks)
skilld author eject vue --name vue  # Eject with custom skill dir name
skilld author validate <file>       # Validate a skill section
skilld author assemble [dir]        # Merge enhancement output into SKILL.md
```

## Architecture

CLI tool and curated registry for AI agent skills. Requires Node >= 22.6.0. Primary flow: `skilld add npm:<pkg>` → fetch curated skill from skilld.dev → install to agent dirs. Fallback: `skilld author package <pkg>` → resolve docs → cache references → generate SKILL.md.

**Key directories:**
- `~/.skilld/` - Global cache: `references/<pkg>@<version>/`, `llm-cache/`, `config.yaml`
- `.claude/skills/<pkg>/SKILL.md` - Generated skill files (project-level)
- `src/commands/` - CLI subcommands routed via citty `subCommands` in cli.ts
- `src/agent/` - Agent registry, detection, LLM spawning, skill generation
  - `clis/` - LLM CLI integrations (claude, codex, gemini)
  - `prompts/` - Skill generation prompt templates; `optional/` for toggleable sections (api-changes, best-practices, types, etc.)
  - `targets/` - Per-agent target definitions
- `src/sources/` - Doc fetching (npm registry, llms.txt, GitHub via ungh.cc)
- `src/cache/` - Reference caching with symlinks to `~/.skilld/references/`
- `src/retriv/` - Vector search with sqlite-vec + @huggingface/transformers embeddings
- `src/core/` - Config (custom YAML parser), skills iteration, formatting, lockfile, prefix parser
- `src/registry/` - Registry client for skilld.dev API (curated skill fetching)

**Doc resolution cascade (src/commands/sync.ts):**
1. Package ships `skills/` directory → symlink directly (skills-npm convention)
2. Git-hosted versioned docs → fetch from GitHub tags via ungh.cc
3. Registry `crawlUrl` → crawl specific URL pattern via `@mdream/crawl` (e.g. motion-v)
4. `llms.txt` at package homepage → parse and download linked .md files
5. Website crawl → crawl `docsUrl/**` via sitemap when no docs found above
6. GitHub README via ungh proxy → fallback

Resolution tracked via `ResolveAttempt[]` array for debugging failures. Blog release posts (curated in `src/sources/blog-presets.ts`) supplement docs for major version announcements.

**Git skills (`src/sources/git-skills.ts`, `src/commands/sync-git.ts`):**
Installs pre-authored skills from git repos. Accepts `owner/repo`, full URLs, SSH, or local paths. Clones/pulls into `~/.skilld/git-skills/`, copies `skills/` directory contents. Part of skills-npm ecosystem compatibility.

**LLM integration (NO AI SDK):**
Spawns CLI processes directly (`claude`, `gemini`) with `--add-dir` for references. Custom stream-json parsing for progress. Results cached at `~/.skilld/llm-cache/<sha256>.json` with 7-day TTL.

**Agent detection (`src/agent/detect.ts`):**
Checks env vars (`CLAUDE_CODE`, `CURSOR_SESSION`) and project dirs (`.claude/`, `.cursor/`) to auto-detect target. Registry in `src/agent/registry.ts` defines per-agent skill dirs and detection. Supports 11 agents: claude-code, cursor, windsurf, cline, codex, github-copilot, gemini-cli, goose, amp, opencode, roo.

**Import scanning (`src/agent/detect-imports.ts`):**
AST-based import detection via oxc-parser. `FILE_PATTERN_MAP` in `src/agent/types.ts` maps ~98 packages to file glob patterns (e.g., `vue` → `*.vue`) for efficient scanning. `detect-presets.ts` handles framework-specific package discovery (e.g., Nuxt modules).

**Cache structure:**
```
~/.skilld/references/<pkg>@<version>/
  docs/          # Fetched external docs
  issues/        # Individual issue files (issue-123.md)
  discussions/   # Individual discussion files (discussion-42.md)
  pkg/           # Symlink → node_modules/<pkg>
```
References are global/static; SKILL.md is per-project (different conventions). Cache key is exact `name@version`. Symlinks are created in `.claude/skills/<pkg>/.skilld/` (gitignored, recreated by `skilld install`).

## Conventions

- **Functional only** — no classes, pure functions throughout
- **Custom YAML** — `src/core/yaml.ts` hand-rolled parser (no yaml library). `yamlEscape()` double-quotes values with special chars, `yamlParseKV()` splits on first colon. Used for config.yaml and skilld-lock.yaml
- **Markdown sanitization** — `src/core/sanitize.ts` strips prompt injection vectors (zero-width chars, HTML comments, agent directive tags, external images/links, base64 blobs, directive patterns). Code-fence-aware via state machine
- **Let errors propagate** — fetch errors return `null`, resolution tracks attempts in `ResolveAttempt[]`
- **Parallelization** — `p-limit` for concurrency, batch downloads (20 at a time), `sync-parallel.ts` for multi-package
- **Package registry** — `src/sources/package-registry.ts` unified registry keyed by `owner/repo`, consolidates doc overrides, blog presets, and file patterns for packages with broken/missing npm metadata
- **Version comparison** — `isOutdated()` compares exact versions
- **Tests** — vitest projects (unit + e2e), `globals: true`, tests in `test/unit/` and `test/e2e/`, fs mocked via `vi.mock('node:fs')`. E2E tests include preset workflows (nuxt, vue, react, svelte, etc.) in `test/e2e/preset-*.test.ts`
- **Build** — `obuild` bundles entry points: `index`, `cli`, `cli-entry`, `prepare`, `types`, and `retriv/worker` (worker spawned by `retriv/pool.ts` at runtime). Package `exports` declares only `.`; nothing else is a published subpath.
- **CLI modes** — `skilld update -b` for pnpm prepare hooks (background, non-interactive, auto-uses configured model). `skilld eject` exports portable skills with references as real files. `--debug` saves raw LLM output to `logs/`
- **First-run wizard** — `src/commands/wizard.ts` handles agent/model config and package selection on first run
- **E2E agent tests** — `test/e2e-agents/` validates skill generation across multiple agents via `generate-matrix.ts` and `generate-pipeline.ts`

<!-- skilld -->
Before modifying code, evaluate each installed skill against the current task.
For each skill, determine YES/NO relevance and invoke all YES skills before proceeding.
<!-- /skilld -->
