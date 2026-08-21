---
name: skilld
description: Search, view, install, upgrade, verify, and remove Skills with skilld CLI, including private repository access.
---

# Use skilld CLI

Use skilld CLI to search for and install Skills.

## Search for a Skill

Run a focused search:

```sh
skilld search <query> --json
```

Read `data.items` before choosing a Skill.
Use each item's `selector` for install.
Refine the query when several Skills cover different tasks.

Always use `--json` when an Agent runs Skill search.
Check the exit code before reading stdout.
If search fails, read the tagged JSON error from stderr.
Use `--plain` only when another command needs stable text.
Never parse formatted terminal output.

## Install a Skill

Install the selector returned by search into the detected Agent target:

```sh
skilld install <selector>
```

Install into global Agent targets:

```sh
skilld install <selector> --global
```

Install this skilld-maintained Skill globally:

```sh
skilld install skilld --global
```

Always use the source selector shown by `skilld search`.
If private repository access is required, run `skilld auth login`.
Do not print access tokens or copy them into project files.

After installation, report the installed Skill name and Agent target.

## View a Skill

```sh
skilld list
skilld view <skill>
```

Use `list` to show installed Skills.
Use `view` to show one Skill's path, source status, and Agent targets.

## Maintain installed Skills

```sh
skilld upgrade <skill>
skilld verify <skill>
skilld remove <skill>
```

Use `upgrade` to install a newer Artifact.
Use `verify` to check the installed bytes and source status.
Use `remove` only when the request names the Skill to remove.
