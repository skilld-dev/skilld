---
type: tool_used
tool: Bash
input_match: "(npm|pnpm|yarn) pack|(npm|pnpm|yarn) (install|i|add) [^\\n]*(\\.tgz|file:|/project)"
min: 1
weight: 1
---

The run tests the examples in a consumer fixture that installs the package,
not by importing the package source directly.
