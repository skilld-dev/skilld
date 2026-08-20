---
name: review-skill
description: Review an Agent Skill for valid structure, clear triggers, usable instructions, current evidence, and risky or unclear actions.
---

# Review a Skill

Review the supplied Skill as an Agent would use it.

## Checks

1. Confirm `SKILL.md` exists and its parent directory matches its name.
2. Confirm frontmatter uses supported fields and valid values.
3. Confirm the description states clear trigger conditions.
4. Follow every linked reference and script.
5. Report missing or broken links.
6. Reject symbolic links, special files, and paths that leave the Skill directory.
7. Check instructions for missing inputs, unclear outcomes, and silent failure paths.
8. Check commands for destructive scope, credential exposure, and unverified downloads.
9. Check examples against the cited API or project source.
10. Find repeated prose and material that belongs in a reference.

Rank each finding as `error`, `warning`, or `note`.
Give the exact path and a direct fix.
Do not rewrite the Skill unless the request asks for changes.

For a direct run, present the findings to the user.
The user decides whether to apply them.
