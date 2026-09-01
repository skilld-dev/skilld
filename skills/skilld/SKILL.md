---
name: skilld
description: Operate skilld CLI for Skill discovery, use, installation, inspection, updates, authentication, configuration, restoration, and removal.
---

# Use skilld CLI

Use skilld CLI to find, load, install, inspect, restore, update, verify, and remove Skills.

Run a Skill first. Install a Skill only when the user asks to keep it.

## Use Agent output

Use `--json` with `search`, `run`, and `update --check`.
These are the only commands that support JSON output.
Use `--plain` when another command needs stable text.

Check the exit code before reading stdout.
Read JSON success data only when `_tag` is `Success`.
Read JSON failures from stderr.
Report the error `code` and `message`.

An update check can exit with code 1 and return valid JSON.
Read its update relations before treating that exit as a failure.
Never parse formatted terminal output.

## Search for a Skill

Run a focused search:

```sh
skilld search <query> --json
```

Read `data.items` before choosing a Skill.
Use each item's `selector` for a Skill run.
Refine the query when several Skills cover different tasks.

Do not guess a selector from the Skill name.
Do not install a search result before reading its description.

## Run a Skill

Run the selector returned by search:

```sh
skilld run <selector> --json
```

The command prints SKILL.md and writes no Skill files.
It retains no remote Skill files after the command exits.
Read the printed SKILL.md, then follow it for the current task.
Prefer `skilld run` for a one-off task.

Read `data.files` for each supporting file's path, kind, and size.
The initial load prints no supporting file content.
Read one only when the instructions name it:

```sh
skilld run <selector> --revision <data.revision> --file <path> --json
```

Run the exact `data.files[].readArgv` array when possible.
It contains the source, exact revision, file path, and `--json`.
Repeat `--file` to read several files in one command.

Check `data.files[].readable` before you ask for a file.
A file with `readable: false` never prints.
Its `kind` is `executable` or `binary`.
Tell the user the Skill needs an install to use that file.

Report which Skill you ran and that skilld wrote no Skill files.
Read `data.sourceStatus`, `data.origin`, and `data.revision`.
A `verified` status covers where the Skill came from.
It does not cover what the instructions ask you to do.
If the status is `unverified`, tell the user before you follow the Skill.

## Choose the source

Prefer the exact `skilld:` selector returned by Skill search.
Hosted selectors use immutable artifact delivery from an exact Git commit.

Use a local path only for a Skill the user already controls:

```sh
skilld run ./skills/my-skill --json
skilld install ./skills/my-skill
```

Use `--direct` only for an explicit public GitHub selector.
Direct mode bypasses artifact delivery and gives the `unverified` source status.
It cannot access a private repository.
Never add `--direct` merely to bypass a delivery failure.

Private repository delivery requires a skilld.dev account and GitHub App access.
If private access fails, check authentication before changing the selector.

## Install a Skill

Install a Skill when the user wants it in every session.
Install a Skill when it must run its own script.
Ask the user before you install. An install writes files they did not request.

Install the selector returned by search into the detected Agent target:

```sh
skilld install <selector>
```

The default scope is the current project.
Project installs update `.skills/skilld-lock.yaml` and selected Agent targets.

Install into global Agent targets:

```sh
skilld install <selector> --global
```

Use `--agent <agent>` when the user names an Agent target.
Repeat `--agent` when the user names several Agent targets.
Do not guess a target when detection and `agent.targets` are empty.

Use `--mode copy` or `--mode symlink` only when the user chooses a mode.
Otherwise, use the configured `install.mode`.

Install this skilld-maintained Skill globally:

```sh
skilld install skilld --global
```

Always use the source selector shown by `skilld search`.
After installation, report the Skill name, scope, Agent targets, and source status.

## Restore locked Skills

Restore the current project from its lockfile:

```sh
skilld install
```

Restore global Skills from the global scope:

```sh
skilld install --global
```

A verified remote Skill restores its exact locked commit through artifact delivery.
An unverified remote Skill requires the recovery command shown by skilld.
Do not convert a verified source to direct mode during recovery.

Never delete a lockfile or Agent target to repair an install.
Preserve the files and report the exact failure first.

## Inspect installed Skills

```sh
skilld list
skilld list --global
skilld view <skill>
skilld view <skill> --global
```

Use `list` to find installed names in one scope.
Use `view` to inspect a Skill before any mutation.
Read its path, source, source status, and Agent targets.

## Check and apply updates

```sh
skilld update --check --json
skilld update <skill>
skilld update <skill> --global
```

Use `update --check --json` to inspect update relations without changing files.
Read each `data.items[].relation._tag` before changing files.
Use `update <skill>` only when the relation is `available`.
Treat `current`, `pinned`, and `notTracked` as no action.
If the relation is `behind` or `diverged`, ask before changing files.
If the relation is `unavailable`, report `failure.code` and `failure.message`.
Treat `unavailable` as unknown. Do not infer a newer commit.

Update one named Skill unless the user explicitly requests all updates.
Use `--global` only for a Skill in the global scope.
Leave `--interactive` to a human terminal session.

## Verify source integrity

```sh
skilld verify <skill>
```

Use `verify` to check installed bytes against recorded source data.
A successful check confirms provenance and integrity only.
It does not approve the Skill instructions.

If verification fails, do not hand edit a managed Skill.
Use `view` to inspect its source before update or restore.

## Report outdated and unmanaged Skills

Check the current scope:

```sh
skilld outdated --plain
```

Check both scopes and every Agent target directory:

```sh
skilld outdated --all --plain
```

Use `outdated` for stale, unverified, local, and unmanaged Skill reports.
Read every proposed command before using it.
Never delete an unmanaged Skill unless the user names it for removal.

## Manage account authentication

Check account authentication before starting login:

```sh
skilld auth status --plain
```

Start login only when private artifact delivery requires it:

```sh
skilld auth login --plain
```

Private repository access also requires the skilld GitHub App installation.
Credentials stay in the operating system keychain.
Never print access tokens or copy them into files.

Log out only when the user explicitly asks:

```sh
skilld auth logout --plain
```

## Manage configuration

Read account level configuration before changing it:

```sh
skilld config list --plain
skilld config get agent.targets --plain
skilld config get install.mode --plain
```

Only `agent.targets` and `install.mode` are supported keys.
Set a key only when the user explicitly requests a persistent default.

```sh
skilld config set agent.targets codex,claude-code --plain
skilld config set install.mode copy --plain
```

Valid install modes are `copy` and `symlink`.
Configuration changes affect later commands across projects.

## Remove a Skill

Inspect the named Skill and its scope before removal:

```sh
skilld view <skill> --plain
skilld remove <skill> --plain
```

Add `--global` to both commands for a global Skill.
Remove only the Skill and scope the user names.
Report the removed Agent targets and whether recovery needs a reinstall.

## Handle failures

Preserve the original error code and message.
Do not hide a failure with a fallback source or scope.
Do not retry with `--direct` because it changes the source status.

For authentication errors, run `skilld auth status` before login.
For target errors, inspect `agent.targets` and the requested `--agent` values.
For lockfile errors, preserve the lockfile and report its path.
For target conflicts, stop before overwriting existing files.

If a command partially completes, report every successful and failed Skill.
Never claim success from generated commands that were not run.
