# Harness request

The current Skill is at `{{CURRENT_SKILL_PATH}}`.
The prepared package source for the new version is at `{{SOURCE_PATH}}`.
Write the updated Skill at `{{OUTPUT_PATH}}`. The Skill directory name must be `{{SKILL_NAME}}`.

Read this Skill fully before writing files.
Copy the current Skill to `{{OUTPUT_PATH}}` first. If no change is relevant, leave it byte-identical and finish.
The network allows the npm registry and GitHub source archives only. Fetch the tested version from npm for the diff.
Build fixtures in a scratch directory. Write no other files outside `{{OUTPUT_PATH}}`.
Write only `SKILL.md` and Markdown files under `references/`: at most 9 files and 64 KiB in total. Skillgen rejects other output.
Do not install or run `skilld`. After you finish, the Harness checks the output and returns each failed check to you.
List each untested affected claim and each documentation mismatch in your final message.
