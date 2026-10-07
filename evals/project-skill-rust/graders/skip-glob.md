---
type: regex
target:
  source: file
  path: skills/ledgerd/SKILL.md
pattern: "--glob[ =]['\"]?!|-g[ =]['\"]?!"
match: contains
weight: 1
---

The Skill names the directories to skip in its search command.
