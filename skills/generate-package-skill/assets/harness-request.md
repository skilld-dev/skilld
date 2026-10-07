# Harness request

Use the prepared package source at `{{SOURCE_PATH}}`.
If it exists, use the current Skill at `{{CURRENT_SKILL_PATH}}` as the update baseline.
Generate the package Skill at `{{OUTPUT_PATH}}`.
The Skill directory name must be `{{SKILL_NAME}}`.

Read this Skill fully before writing files.
Use only the visible prepared source and cited official documentation.
Pin external documentation to the dependency versions in the prepared manifest.
If the session cannot run an example, list it as untested in your final message.
List each documentation and behaviour mismatch in your final message.
Write no files outside `{{OUTPUT_PATH}}`. Build fixtures in a scratch directory outside it.
Write only `SKILL.md` and Markdown files under `references/`: at most 9 files and 64 KiB in total. Skillgen rejects other output.
Finish only after checking every output rule in this Skill.
After you finish, the Harness checks the output. It returns each failed check to you; fix them in place.
