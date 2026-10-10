---
name: review-skill
description: Review an Agent Skill for valid structure, clear triggers, usable instructions, current evidence, and risky or unclear actions.
license: MIT
compatibility: "Requires an agent with filesystem and shell access. Network access is needed for remote sources."
---

# Review a Skill

Review the supplied Skill as an Agent would use it.

## Checks

1. Confirm `SKILL.md` exists and its parent directory matches its name.
2. Confirm frontmatter uses supported fields and valid values.
3. Confirm the description says what the Skill does and when to use it, in the third person, with the terms a user types.
4. Follow every linked reference and script.
5. Report missing or broken links.
6. Reject symbolic links, special files, and paths that leave the Skill directory.
7. Check instructions for missing inputs, unclear outcomes, and silent failure paths.
8. Check commands for destructive scope, credential exposure, and unverified downloads.
9. Check examples against the cited API or project source. If a runtime is available, run them and report each result that differs from the claim.
   An example is each code block and each checkable prose claim: a default, a list, a count, error text, an exit code, or "X happens when Y".
   Test the exact version the Skill names, never the repository head. Use the toolchain its users run, in each mode the claim covers, such as SSR, dev, and build.
   Install it in an empty scratch directory with its own `pnpm-workspace.yaml`. Prefer a local fixture to a live site. Never use personal credentials.
   For cloud-bound code, typecheck it and run it against in-memory stand-ins. Confirm a failing claim with a second probe before you report it.
   Mark each example executed, read from source, or unverified.
10. Find repeated prose and material that belongs in a reference.
11. Find text the reader already knows: domain or framework explanations, generic debug advice, changelog paraphrase, and internals the reader cannot act on.
12. For a package Skill, confirm the body names the package version it was tested against, once, in the first paragraph. Flag other version literals.
13. Confirm the body covers each export, option, or symptom the description promises.

Rank each finding:

- `error`: following the Skill gives a wrong result, a failure, or a security gap.
- `warning`: an Agent would act wrongly on the claim, a failure stays silent, or a needed input is missing.
- `note`: precision, repetition, structure, or token cost.

Give the exact path, the line, and a direct fix.
Do not rewrite the Skill unless the request asks for changes.

For a direct run, present the findings to the user.
The user decides whether to apply them.
