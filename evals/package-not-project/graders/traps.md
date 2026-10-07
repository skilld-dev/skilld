---
type: regex
target:
  source: file
  path: skills/tinyfmt/SKILL.md
pattern: "^#{2,3} [^\\n]*[Tt]raps?\\b"
flags: m
match: contains
weight: 1
---

The Skill has a traps section for silent failures and plausible wrong calls.
