---
type: regex
target: trace
pattern: '"--revision","([0-9a-f]{40})"[\s\S]*?\[tool bash\][^\n]*--revision \1[^\n]*--file[= ]onboarding\.md'
match: contains
weight: 2
---
