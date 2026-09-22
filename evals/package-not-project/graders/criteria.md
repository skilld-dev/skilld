---
type: llm
weight: 2
focus: last_message
---

The request is for a Skill that teaches consumers to use the `tinyfmt` package,
not for a Skill that navigates this workspace.

A passing run:

- Follows `generate-package-skill`, and says so.
- Writes consumer-facing instructions: the public export `formatMoney`, its
  arguments, and how to import it.
- Does not turn the result into a project navigation Skill. A layout table of
  this repository, or search commands for changing this repository, is the wrong
  shape here.

It fails if it follows `generate-project-skill`, or if the Skill it writes reads
as internal navigation for this workspace rather than usage for a consumer.
