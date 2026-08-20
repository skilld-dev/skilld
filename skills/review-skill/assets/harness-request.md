# Harness request

Review the prepared Skill at `{{SOURCE_PATH}}`.
Write the result to `{{OUTPUT_PATH}}/review.json`.

Read this Skill fully before reviewing.
Write no other output files.
Use this JSON shape:

```json
{
  "summary": "One short conclusion.",
  "findings": [
    {
      "level": "error",
      "path": "SKILL.md",
      "message": "One clear problem.",
      "fix": "One direct fix."
    }
  ]
}
```

Use only `error`, `warning`, or `note` for `level`.
