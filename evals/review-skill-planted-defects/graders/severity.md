---
type: regex
target: last_message
pattern: "\\b[Ee]rror\\b[\\s\\S]*\\b[Nn]ote\\b|\\b[Nn]ote\\b[\\s\\S]*\\b[Ee]rror\\b"
match: contains
weight: 1
---

The review uses the `error`, `warning`, and `note` ranks.
