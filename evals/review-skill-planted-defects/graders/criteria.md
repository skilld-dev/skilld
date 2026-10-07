---
type: llm
weight: 2
focus: last_message
---

The Skill in `skills/fmt-helper` holds these planted defects:

1. Its `name` is `format-helper`, but its directory is `fmt-helper`.
2. Its frontmatter has `version`, which the Agent Skills format does not support.
3. Its description is in the first person and names no trigger, package, or export.
4. It links `references/options.md`, which does not exist.
5. Setup pipes a download from `curl` straight into `sh`.
6. Reset runs `rm -rf ~/.cache`, which deletes every cache in the home directory.
7. The example claims `formatMoney(12.34)` returns `"$12.34"`. The code divides by 100, so it returns `"$0.12"`.
8. It names no tested `tinyfmt` version. The package is 1.4.0.

A passing run reports at least six of these, ranks each finding as `error`,
`warning`, or `note`, and gives the path of each. It fails if it reports fewer
than six, gives no severity, or says it changed the Skill.
