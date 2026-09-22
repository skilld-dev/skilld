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

## What each case holds

- `project-skill-real-paths`: a TypeScript project with a `dist/` decoy. Checks
  that the Skill names real paths and gives a search the Agent can repeat.
- `project-skill-rust`: a Cargo project with no `package.json`. Checks that the
  Skill reads the project's own manifest and finds its declared binary.
- `package-not-project`: a published package. Checks that a request for consumer
  instructions routes to `generate-package-skill`.

## Write a case

A grader reads the transcript, not the files the Agent wrote. Ask the prompt for
a closing summary that states what a grader needs to see.

An `llm` grader judges shape and intent. A `regex` or `file_exists` grader
settles a fact. Deterministic rules about SKILL.md content belong in the Harness
instead, where `packages/harness/src/internal/output/project.ts` fails a Skill
before promotion.
