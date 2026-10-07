# skilld-maintained Skill evals

These cases score the Skills in `skills/` against scaffolded projects.

Direct Skill runs have no enforcement. An Agent reads the Skill and writes files.
These cases measure what the Agent actually produces.

## Run them

```sh
./scripts/eval-skills.sh
./scripts/eval-skills.sh --case project-skill-rust --runs 1
```

Each run is a real Agent session on your own credential. The suite costs money,
so it never runs in `pnpm test` or in CI on a pull request.

The script stages `skills/`, `evals/`, and `.claude-plugin/` into a temporary
directory before it runs. The repository holds `target/` and `node_modules/`,
which overflow the eval's argument list.

## Run them with opencode

`scripts/eval-opencode.ts` runs the same cases with opencode on a cheap model.
The default model is `zai-coding-plan/glm-5.3-flash`, a flat-rate plan, so a run adds no cost.

```sh
pnpm eval:opencode
pnpm eval:opencode --case 'skilld-*' --arm with
pnpm eval:opencode --cli target/debug/skilld --site https://preview.skilld.dev
```

| Flag | Default | Use |
| --- | --- | --- |
| `--case` | every case | Glob over case names |
| `--arm` | `both` | `with`, `without`, or `both` |
| `--runs` | `1` | Runs per arm. The runner ignores `runs` in `case.yaml` |
| `--model`, `--judge-model` | GLM 5.3 Flash | Any model your opencode config can reach |
| `--no-llm` | off | Skip `llm` graders |
| `--cli` | `skilld` on PATH | The skilld executable under test, such as a local build |
| `--site` | `https://skilld.dev` | Origin for `SKILLD_API_URL` and for `{site}` in a case's `opencode.json` |
| `--concurrency` | `2` | Runs at once |

Each run gets a temp project, a temp `HOME`, a temp opencode config, data, and state home, and a temp `SKILLD_DATA_DIR`.
opencode then loads no Skill from `~/.config/opencode`, `~/.claude/skills`, or `~/.agents/skills`.
The temp `HOME` also stops the Agent from reading an installed Skill with a shell command.
An earlier baseline run did that, then loaded `generate-package-skill` with `skilld run`.
rustup, cargo, npm, and opencode keep their real toolchain and cache directories.
The with arm copies the Skills in `skills/skilld-maintained-skills.json` to `.opencode/skills/`.
Provider settings and credentials pass in memory, so no secret reaches the temp directory.

The runner reads the `claude plugin eval` grader schema.
Name tools as Claude Code does: `Bash`, `Skill`, or `mcp__skilld__search_skills`.
To check a shell command, use `tool_used` with `tool: Bash` and `input_match`. Add `max: 0` for a command that must not run.
`regex` supports `contains` and `not_contains`, but not `count:N`.

Cases that need opencode features live in `evals-opencode/`, so `claude plugin eval` skips them.
An `opencode.json` beside `case.yaml` merges into the run's config. `mcp-find-skill` uses it to connect the skilld MCP server.

Results land in `evals/results/opencode/<time>/`: `report.md`, `summary.json`, and each run's opencode events.
The report's Friction section lists failed commands, skilld error codes, `Next step:` lines, repeated commands, and failed tool calls.
Read it to find where the Skill, the CLI, the API, or the MCP server misled the Agent.

## Read the score

Every case runs twice: once with the Skills loaded, once without. The delta is
the value the Skills add. A case that scores the same in both arms measures the
model, not the Skill.

Recorded on 2026-09-22, 2 runs per arm, Claude Code 2.1.278:

| Case | With | Without | Δ |
| --- | --- | --- | --- |
| `package-not-project` | 1.00 | 0.67 | +0.33 |
| `project-skill-real-paths` | 1.00 | 0.75 | +0.25 |
| `project-skill-rust` | 1.00 | 0.75 | +0.25 |

`package-skill-docs-mismatch` was recorded on 2026-09-29 with 1 run per arm: with 1.00,
without 0.40, Δ +0.60. In two earlier baseline runs the model also ran the code and
reported the mismatch, so treat this delta as noisy until more runs exist.

## What each case holds

- `project-skill-real-paths`: a TypeScript project with a `dist/` decoy. Checks
  that the Skill names real paths and gives a search the Agent can repeat.
- `project-skill-rust`: a Cargo project with no `package.json`. Checks that the
  Skill reads the project's own manifest and finds its declared binary.
- `package-not-project`: a published package. Checks that a request for consumer
  instructions routes to `generate-package-skill`.
- `package-skill-docs-mismatch`: a package whose README documents dollars while
  the code takes cents. Checks that the Skill runs its examples, writes the
  observed behaviour, and reports the mismatch to the maintainer.

The `skilld` Skill cases check the CLI workflow it teaches:

- `skilld-run-not-install`: a one-off task searches, runs a Skill, and installs nothing.
- `skilld-install-when-asked`: a request to keep a Skill installs the exact selector for opencode.
- `skilld-missing-ref`: a missing Skill reports the skilld error code and invents nothing.
- `skilld-trending`: a trending question reads `skilld trending --json` and reports why each Skill trends.
- `skilld-provenance`: a provenance question reads `skilld view` and names the exact SKILL.md.
- `evals-opencode/mcp-find-skill`: with the skilld MCP server connected, the Agent finds a Skill with `search_skills` and returns a run command.
- `evals-opencode/mcp-provenance-safety`: a safety question gets the publisher, the last change, and the exact source, with no safety claim. The Skill fetches its rules from a remote URL, and the answer must say so.

The generator cases also check rules a baseline misses. Each grader reads the written `SKILL.md` or the commands the run used:

- Project Skills: a search command that names the directories to skip, a note on which changes need a new Skill run, and the project root as the working directory.
- Package Skills: examples tested in a consumer fixture that installs the package, each run wrapped in `timeout`, and a traps section.
- `review-skill-planted-defects`: a Skill with eight planted defects. The review must find six, rank each as `error`, `warning`, or `note`, give its path, and leave the Skill unchanged.

### opencode results

Recorded on 2026-10-07 with `zai-coding-plan/glm-5.3-flash` as the Agent and the judge, opencode 1.18.32, and skilld 3.6.2.
The `skilld-*` cases ran once per arm. Every other case ran twice per arm.

| Case | With | Without | Δ |
| --- | --- | --- | --- |
| `package-not-project` | 1.00 | 0.50 | +0.50 |
| `package-skill-docs-mismatch` | 1.00 | 0.50 | +0.50 |
| `project-skill-real-paths` | 0.86 | 0.57 | +0.29 |
| `project-skill-rust` | 1.00 | 0.57 | +0.43 |
| `review-skill-planted-defects` | 1.00 | 0.88 | +0.13 |
| `skilld-install-when-asked` | 1.00 | 0.00 | +1.00 |
| `skilld-missing-ref` | 1.00 | 0.83 | +0.17 |
| `skilld-provenance` | 1.00 | 1.00 | +0.00 |
| `skilld-run-not-install` | 1.00 | 1.00 | +0.00 |
| `skilld-trending` | 1.00 | 0.20 | +0.80 |
| `mcp-find-skill` | 1.00 | 1.00 | +0.00 |
| `mcp-provenance-safety` | 0.71 | 0.71 | +0.00 |

The two project rows come from a later with-arm rerun. The first run's judge failed a Skill for naming `.opencode/`, the directory where the runner puts the eval's Skills. The criteria now allow it.

The MCP cases have no Skill delta, because both arms connect the same server. They measure the server:

- The MCP server no longer serves curator collections (skilld-dev/skilld.dev#548), so the `mcp-curator-collection` case was removed. It failed in every run: the Agent guessed 15 to 30 slugs and never found `agent-building-stack`.
- `mcp-provenance-safety` fails its criteria in every run. Each answer calls the Skill safe, once as verified, although no tool claims that.

`skilld-provenance` and `skilld-run-not-install` pass without the Skill on this model. `skilld --help` is enough for them.

## Write a case

A grader reads the transcript, not the files the Agent wrote. Ask the prompt for
a closing summary that states what a grader needs to see.

An `llm` grader judges shape and intent. A `regex` or `file_exists` grader
settles a fact. Deterministic rules about SKILL.md content belong in the Harness
instead, where `packages/harness/src/internal/output/project.ts` fails a Skill
before promotion.
