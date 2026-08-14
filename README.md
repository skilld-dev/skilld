<h1><a href="https://skilld.dev"><img src=".github/logos/logo-mark.svg" alt="" width="28" height="28" valign="middle"></a> skilld</h1>

[![npm version](https://img.shields.io/npm/v/skilld?color=yellow)](https://npmjs.com/package/skilld)
[![npm downloads](https://img.shields.io/npm/dm/skilld?color=yellow)](https://npm.chart.dev/skilld)
[![license](https://img.shields.io/npm/l/skilld?color=yellow)](https://github.com/skilld-dev/skilld/blob/main/LICENSE)

> Generate AI agent skills from your NPM dependencies.

## Why?

When using new packages or migrating to new versions, agents often struggle to use the appropriate best practices. This is because
agents have [knowledge cutoffs](https://platform.claude.com/docs/en/about-claude/models/overview#latest-models-comparison) and
predict based on existing patterns.

Methods of getting the right context to your agent require either manual curation, author opt-in, external servers or vendor lock-in. See [the landscape](#the-landscape)
for more details.

Skilld generates [agent skills](https://agentskills.io/home) from the references maintainers already create: docs, release notes and GitHub issues. With these we can create version-aware, local-first, and optimized skills.

<p align="center">
<table>
<tbody>
<td align="center">
<sub>Made possible by my <a href="https://github.com/sponsors/harlan-zw">Sponsor Program 💖</a><br> Follow me <a href="https://twitter.com/harlan_zw">@harlan_zw</a> 🐦 - Join <a href="https://discord.gg/275MBUBvgP">Discord</a> for help</sub><br>
</td>
</tbody>
</table>
</p>

## Features

- 🌍 **Any Source: Opt-in** - Any NPM dependency or GitHub source, docs auto-resolved
- 📦 **Bleeding Edge Context** - Latest issues, discussions, and releases. Always use the latest best practices and avoid deprecated patterns.
- 📚 **Opt-in LLM Sections** - Enhance skills with LLM-generated `Best Practices`, `API Changes`, or your own custom prompts
- 🧩 **Use Any Agent** - Choose your agent: CLI , [pi](https://github.com/badlogic/pi-mono/tree/main/packages/ai) agents or no agent at all.
- 🔍 **Semantic Search** - Query indexed docs across all skills via [retriv](https://github.com/harlan-zw/retriv) embeddings
- 🧠 **Context-Aware** - Follows [Claude Code skill best practices](https://code.claude.com/docs/en/skills#add-supporting-files): SKILL.md stays under 500 lines, references are separate files the agent discovers on-demand - not inlined into context
- 🎯 **Safe & Versioned** - Prompt injection sanitization, version-aware caching, auto-updates on new releases
- 🤝 **Ecosystem** - Compatible with [`npx skills`](https://skills.sh/) and [skills-npm](https://github.com/antfu/skills-npm). Skilld auto-detects and uses skills-npm packages when available.

## Quick Start

Run skilld in a project to generate skills for your dependencies through a simple interactive wizard:

```bash
npx -y skilld
```

__Requires Node 22.6.0 or higher.__

Or add a specific package directly:

```bash
npx -y skilld add npm:vue
```

If you need to re-configure skilld, just run `npx -y skilld config` to update your agent, model, or preferences.

**No agent CLI?** No problem - choose "No agent" when prompted. You get a base skill immediately, plus portable prompts you can run in any LLM:

```bash
npx -y skilld add npm:vue
# Choose "No agent" -> base skill + prompts exported
# Paste prompts into ChatGPT/Claude web, save outputs, then:
npx -y skilld author assemble
```

### Tips

- **Be selective** - Only add skills for packages your agent struggles with. Not every dependency needs one.
- **LLM is optional** - Skills work without any LLM, but enhancing with one makes them significantly better.
- **Multi-agent** - Run `skilld install --agent gemini-cli` to sync skills to another agent. The doc cache is shared.

## Installation

### Global

Install globally to use `skilld` across all projects without `npx`:

```bash
npm install -g skilld
# or
pnpm add -g skilld
```

Then run `skilld` in any project directory.

### Per-Project

If you'd like to install skilld and track the lock file references, add it as a dev dependency:

```bash
npm install -D skilld
# or
yarn add -D skilld
# or
pnpm add -D skilld
```

### Automatic Updates

Add to `package.json` to restore references and sync shipped skills on install:

```json
{
  "scripts": {
    "prepare": "skilld prepare"
  }
}
```

This restores symlinks, auto-installs shipped skills from your deps, and notifies you when packages have new features or breaking changes. Run `skilld update` to regenerate LLM enhancements.

## FAQ

### Why don't the skills run?

Try this in your project/user prompt:

```md
Before modifying code, evaluate each installed skill against the current task.
For each skill, determine YES/NO relevance and invoke all YES skills before proceeding.
```

### How is this different from Context7?

Context7 is an MCP that fetches raw doc chunks at query time. You get different results each prompt, no curation, and it requires their server. Skilld is local-first: it generates a SKILL.md that lives in your project, tied to your actual package versions. No MCP dependency, no per-prompt latency, and it goes further with LLM-enhanced sections, prompt injection sanitization, and semantic search.

### Will I be prompt injected?

Skilld pulls issues from GitHub which could be abused for potential prompt injection.

Skilld treats all data as untrusted, running in permissioned environments and using best practices to avoid injections.
However, always be cautious when using skills from untrusted sources.

### Do skills update when my deps update?

Yes. Add `skilld prepare` to your prepare script. It restores references, auto-installs shipped skills, and tells you when packages are outdated. Run `skilld update` to regenerate LLM enhancements.

## CLI Usage

```bash
# Interactive mode - auto-discover from package.json
skilld

# Add skills for specific package(s) — npm: prefix for registry packages
skilld add npm:vue npm:nuxt npm:pinia

# The same prefixes work in the interactive wizard's package prompt

# Add a pre-authored skill from a GitHub repo
skilld add gh:vercel-labs/agent-skills

# Add a skill for a Rust crate (crates.io)
skilld add crate:serde

# Update outdated skills
skilld update
skilld update tailwindcss

# Build a searchable skill from the current project
skilld self
skilld search "how is authentication handled" -p self

# Search docs across installed skills
skilld search "useFetch options" -p nuxt
skilld search "error" -p nuxt --filter '{"type":"issue"}'
skilld search "routing" --agents claude-code
skilld search --guide -p nuxt

# Target a specific agent
skilld add npm:react --agent cursor

# Install globally to ~/.claude/skills
skilld add npm:zod --global

# Skip prompts
skilld add npm:drizzle-orm --yes

# Check skill info
skilld info

# List installed skills
skilld list
skilld list --json

# Manage settings
skilld config
```

### Commands

| Command | Description |
|---------|-------------|
| `skilld` | Interactive wizard (first run) or status menu (existing skills) |
| `skilld add <source...>` | Add skills. Sources: `npm:<pkg>`, `crate:<name>`, `gh:<owner/repo>`, or bare names (deprecated) |
| `skilld update [pkg]`   | Update outdated skills (all or specific) |
| `skilld self`           | Build a searchable skill from the current project source and docs |
| `skilld search [query]` | Search indexed docs (`-p` package, `--agents` filter, `--filter` JSON, `--limit`, `--guide`) |
| `skilld list`           | List installed skills (`--json` for machine-readable output) |
| `skilld info`           | Show skill info and config |
| `skilld config`         | Configure agent, model, preferences |
| `skilld install`        | Restore references from lockfile |
| `skilld remove`         | Remove installed skills |
| `skilld uninstall`      | Remove all skilld data |
| `skilld cache`          | Cache management (clean expired LLM cache entries) |
| `skilld author package <pkg>`  | Generate a portable package skill from docs |
| `skilld author publish` | Publish skills to skilld.dev |
| `skilld author eject <pkg>`    | Eject skill as portable directory (no symlinks) |
| `skilld author validate <file>`| Validate a skill section |
| `skilld author assemble [dir]` | Merge LLM output files back into SKILL.md (auto-discovers) |

### Works Without an Agent CLI

No Claude, Gemini, or Codex CLI? Choose "No agent" when prompted. You get a base skill immediately, plus portable prompts you can run in any LLM to enhance it:

```bash
skilld add npm:vue
# Choose "No agent" -> installs to .claude/skills/vue-skilld/

# What you get:
#   SKILL.md           <- base skill (works immediately)
#   PROMPT_*.md        <- prompts to enhance it with any LLM
#   references/        <- docs, issues, releases as real files

# Run each PROMPT_*.md in ChatGPT/Claude web/any LLM
# Save outputs as _BEST_PRACTICES.md, _API_CHANGES.md, then:
skilld author assemble
```

`skilld author assemble` auto-discovers skills with pending output files. `skilld update` re-exports prompts for outdated packages.

### Local Models (Ollama)

If [Ollama](https://ollama.com) is running, skilld auto-detects your locally-pulled text models and lists them in `skilld config` (and `-m ollama:<name>`):

```bash
skilld add npm:vue -m ollama:qwen2.5:14b-instruct
```

Generation runs locally: free, offline, no API key. Unlike the CLI and API backends, the Ollama path is one-shot (it does not explore reference files with tools), so all source material is inlined into a single prompt. Best paired with a capable instruct model.

| Env var | Default | Purpose |
|---------|---------|---------|
| `OLLAMA_HOST` | `http://localhost:11434` | Ollama daemon address |
| `OLLAMA_NUM_CTX` | `32768` | Context window; sized to fit the full prompt + output |

The large default context can exceed memory for big models on constrained hardware (Ollama returns a 500). Lower `OLLAMA_NUM_CTX` or pick a smaller model if generation fails to load.

### Embedding Model

Search covers skills installed for every agent in the project, deduplicated. Restrict it with `--agents`:

```bash
skilld search "routing" --agents claude-code
skilld search "routing" --agents claude-code,codex
```

`skilld search` is powered by a local embedding model. It runs offline through transformers.js. It needs no API key or network traffic after the first download. Pick one under **Embedding model** in `skilld config`:

| Model | Dimensions | Notes |
|-------|-----------:|-------|
| `bge-small-en-v1.5` | 384 | Default. Fastest to index, smallest download. |
| `bge-base-en-v1.5` | 768 | Balanced accuracy and speed. |
| `Xenova/bge-large-en-v1.5` | 1024 | Most accurate English retrieval, slowest to index. |
| `bge-m3` | 1024 | Multilingual, 8192-token context. |

Larger models retrieve more accurately but cost more time and memory when indexing. Locally-pulled [Ollama](#ollama-embedding-models) models can be used too. Export `SKILLD_EMBED_MODEL` to override the saved setting for every index and search command in the current shell:

```bash
export SKILLD_EMBED_MODEL=bge-m3
skilld update
```

Search indexes must use one embedding model and device. Rebuild them after switching either setting:

```bash
skilld update
```

### Ollama Embedding Models

If [Ollama](https://ollama.com) is running, locally-pulled embedding models appear in the **Embedding model** picker alongside the built-in ones. They are addressed as `ollama:<name>`, matching the `-m ollama:<name>` syntax used for enhancement models:

```bash
ollama pull qwen3-embedding
SKILLD_EMBED_MODEL=ollama:qwen3-embedding skilld add npm:vue
```

Only models that advertise the `embedding` capability are listed, so chat models cannot be selected by mistake. Dimensions and context length are read from Ollama. skilld validates each vector before indexing.

This talks to Ollama's HTTP API directly. It needs no additional dependency or API key. Set `OLLAMA_HOST` to point at a non-default daemon. If Ollama is not running, the picker simply shows the built-in models.

Ollama manages its own execution device, so **Embedding device** does not apply to `ollama:` models.

### Embedding Device

The embedding model runs on the CPU by default. **Embedding device** in `skilld config` moves it onto a GPU backend, which can be substantially faster:

| Device | Notes |
|--------|-------|
| `auto` | Default. Lets transformers.js choose; CPU under Node. |
| `cpu` | Always available, predictable. |
| `webgpu` | Fastest on Apple Silicon in testing. |
| `coreml` | Apple Neural Engine. Measured slower than CPU for these models. |

Measured on an Apple M5 Max, 120 documents, best of 3 after warm-up (docs/sec):

| Model | `cpu` | `coreml` | `webgpu` |
|-------|------:|---------:|---------:|
| `bge-small-en-v1.5` | 664 | 198 | **1713** |
| `bge-base-en-v1.5` | 198 | 68 | **580** |
| `Xenova/bge-large-en-v1.5` | 71 | 9 | **201** |

WebGPU was 2.6 to 2.9 times faster than CPU at every size. On WebGPU, `bge-large` indexed faster than `bge-base` on CPU and improved retrieval. CoreML was consistently slower.

The ranking is hardware-specific, so benchmark before trusting a device on other machines. Export `SKILLD_EMBED_DEVICE` to override the saved setting in the current shell:

```bash
export SKILLD_EMBED_DEVICE=cpu
skilld update
```

If a backend is unavailable, indexing fails to start. Switch back to `auto`.

### Eject

Export a skill as a portable, self-contained directory for sharing via git repos:

```bash
skilld author eject vue                    # Default skill directory
skilld author eject vue --name vue         # Custom directory name
skilld author eject vue --out ./skills/    # Custom path
skilld author eject vue --from 2025-07-01  # Only recent releases/issues
```

Share via `skilld add gh:owner/repo` - consumers get fully functional skills with no LLM cost.

### CLI Options

| Option | Alias | Default | Description |
|--------|-------|---------|-------------|
| `--global` | `-g` | `false` | Install globally to `~/<agent>/skills` |
| `--agent` | `-a` | auto-detect | Target specific agent (claude-code, cursor, etc.) |
| `--yes` | `-y` | `false` | Skip prompts, use defaults |
| `--force` | `-f` | `false` | Ignore all caches, re-fetch docs and regenerate |
| `--model`      | `-m` | config default | LLM model for skill generation (sonnet, haiku, opus, etc.) |
| `--name`       | `-n` |                | Custom skill directory name (eject only) |
| `--out`        | `-o` |                | Output directory path override (eject only) |
| `--from`       |      |                | Collect releases/issues/discussions from this date (YYYY-MM-DD, eject only) |
| `--debug`      |      | `false`        | Save raw LLM output to logs/ for each section |

## For Maintainers

Ship skills with your npm package so consumers get them automatically. No LLM needed on their end.

Install the official publication workflow for authoring, validation, packaging, and postpublish verification:

```bash
npx skilld add gh:skilld-dev/skills --skill publish-skill --yes
```

The skill orchestrates publication. `skilld author package` remains the deterministic package documentation generator it invokes.

### Generate a skill

From your package root (or monorepo root):

```bash
npx skilld author
```

In a monorepo, skilld auto-detects workspaces and prompts which packages to generate for. Docs are resolved from: package `docs/`, monorepo `docs/content/`, `llms.txt`, or `README.md`.

This creates a `skills/<your-package>/` directory with a `SKILL.md` and ejected reference files. It also adds `"skills"` to your `package.json` `files` array.

### How consumers get it

Once published, consumers run:

```bash
npx skilld prepare
```

Or add it to their `package.json` so it runs on every install:

```json
{
  "scripts": {
    "prepare": "skilld prepare"
  }
}
```

`skilld prepare` auto-detects shipped skills in `node_modules` and symlinks them into the agent's skill directory. Compatible with [skills-npm](https://github.com/antfu/skills-npm).

### Options

| Flag      | Alias | Default | Description |
|:----------|:-----:|:-------:|:------------|
| `--model` | `-m`  |         | LLM model for enhancement |
| `--out`   | `-o`  |         | Output directory (single package only) |
| `--force` | `-f`  | `false` | Clear cache and regenerate |
| `--yes`   | `-y`  | `false` | Skip prompts, use defaults |
| `--debug` |       | `false` | Save raw LLM output to logs/ |

## The Landscape

Several approaches exist for steering agent knowledge. Each fills a different niche:

| Approach | Versioned | Curated | No Opt-in | Local | Any LLM |
|:---------|:---------:|:-------:|:---------:|:-----:|:-------:|
| **Manual rules** | - | yes | yes | yes | yes |
| **llms.txt** | ~ | - | - | - | yes |
| **MCP servers** | yes | - | - | - | - |
| **skills.sh** | - | ~ | yes | - | - |
| **skills-npm** | yes | yes | - | yes | - |
| **skilld** | yes | yes | yes | yes | yes |

> **Versioned** - tied to your installed package version. **Curated** - distilled best practices, not raw docs. **No Opt-in** - works without the package author doing anything. **Local** - runs on your machine, no external service dependency. **Any LLM** - works with any LLM, not just agent CLIs.

- **Manual rules** (CLAUDE.md, .cursorrules): full control, but you need to already know the best practices and maintain them across every dep.
- **[llms.txt](https://llmstxt.org/)**: standard convention for exposing docs to LLMs, but it's full docs not curated guidance and requires author adoption.
- **MCP servers** (Context7, etc.): live, version-aware responses, but adds per-request latency and the maintainer has to build and maintain a server.
- **[skills.sh](https://skills.sh/)**: easy skill sharing with a growing ecosystem, but community-sourced without version-awareness or author oversight.
- **[skills-npm](https://github.com/antfu/skills-npm)**: the ideal end-state: zero-token skills shipped by the package author, but requires every maintainer to opt in. Skilld auto-detects and uses skills-npm packages when available.
- **skilld**: generates version-aware skills from existing docs, changelogs, issues, and discussions. Works for any package without author opt-in.

## Telemetry

Skilld sends anonymous install events to [skills.sh](https://skills.sh/) so skills can be discovered and ranked. No personal information is collected.

Telemetry is automatically disabled in CI environments.

To opt out, set either environment variable:

```bash
DISABLE_TELEMETRY=1
DO_NOT_TRACK=1
```

## Related

- [skills-npm](https://github.com/antfu/skills-npm) - Convention for shipping agent skills in npm packages
- [agentskills.io](https://agentskills.io) - Agent skills specification
- [mdream](https://github.com/harlan-zw/mdream) - HTML to Markdown converter
- [retriv](https://github.com/harlan-zw/retriv) - Vector search with sqlite-vec

## License

Licensed under the [MIT license](https://github.com/skilld-dev/skilld/blob/main/LICENSE).
