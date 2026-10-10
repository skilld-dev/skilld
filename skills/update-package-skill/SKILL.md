---
name: update-package-skill
description: Updates an existing package Skill for a new release of its npm or local package by testing only what changed since the version the Skill names. Use when a maintainer asks to refresh a package Skill after a release, or when a SKILL.md names an older tested version than the package.
license: MIT
compatibility: "Requires an agent with filesystem and shell access. Network access is needed for remote sources."
---

# Update a package Skill

Bring an existing package Skill to a new package version with the smallest correct edit.
The current Skill was tested against an earlier version. Trust what the release did not touch.
To write a Skill from nothing, use `generate-package-skill`.

## Inputs

- The current Skill directory, and the package at the new version: a name and version, a directory, or prepared source.
- The tested version is the version the Skill's first paragraph names. If it names none, stop and report that the Skill needs a full rewrite.
- If the tested version equals the new version, change nothing and report that.

## 1. Copy, then diff

1. Copy the current Skill to the destination unchanged. Edit only the copy.
2. Diff the two published versions: `npm diff --diff=PACKAGE@OLD --diff=PACKAGE@NEW`. For a local package, diff the old tarball against the packed build.
3. Read the changelog or release notes between the two versions.
4. List what changed for a consumer: exports, types, options and their defaults, config keys, error messages, CLI flags, peer ranges, engines, and behaviour the notes describe.

## 2. Map changes to the Skill

For each change, find the Skill lines it affects: a claim, an example, a trap, a version limit.
A change is relevant when it affects a Skill line, or when it adds or breaks something an Agent would get wrong: a new trap, a renamed call, a new required setup step.
Internals, refactors, and fixes the Skill never mentioned are not relevant.

If no change is relevant, leave the copy byte-identical and stop. Report "no change needed" with a one-line diff summary.

## 3. Test the affected lines

1. Create one consumer fixture outside the package source, with its own `pnpm-workspace.yaml`. Install the new version and the documented peers.
2. Run each affected example and claim, and each example you add. Compare output with the claim: return values, type errors, build logs, exit codes.
3. Run each with one failure input as well, such as a bad option or an unreachable URL.
4. Do not re-run untouched examples. The earlier version's tests cover them.
5. Wrap every run in `timeout 120`. Start servers from your own script, record each process ID you start, and stop only those. Never kill by port or with `pkill -f`.
6. Spend about 20 tool calls on testing. Report an affected claim you could not test as untested; never claim it passed.

## 4. Edit

- Change only affected lines, and add what the release needs: a new trap, an old call → new call row, a new common task.
- Delete lines the release made false, and version limits it fixed.
- Update the tested version in the first paragraph. Other version literals go stale; avoid them.
- Keep the wording, order, and formatting of untouched lines exactly. The diff must show only what the release changed.
- Update the description only when a trigger changed, such as a renamed export or a new config key.

Keep the format rules:

- The frontmatter contains `name` and `description`, with optional `license` and `compatibility`. The name matches the directory.
- Preserve the source license. Update `compatibility` when requirements change, keeping it under 501 characters.
- The description is one plain YAML line without double quotes, backticks, or `%`.
- `SKILL.md` stays under 500 lines. Write at most eight reference files, each linked from `SKILL.md`.

## 5. Report

- The tested version, old → new, and the files you changed.
- Each changed line with the diff hunk or release note behind it.
- Each affected claim left untested.
- Each mismatch between the new docs and observed behaviour: example, documented result, observed result. These are package bugs.

For a direct run, show the Skill diff for review. A direct run has no Harness checks; never claim it passed them.
