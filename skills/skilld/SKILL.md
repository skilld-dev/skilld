---
name: skilld
description: Search, view, install, upgrade, verify, and remove Skills with skilld CLI, including private repository access.
---

# Use skilld CLI

Use skilld CLI to search for and install Skills.

## Search for a Skill

Run a focused search:

```sh
skilld search <query>
```

Read the result names and descriptions before choosing.
Refine the query when several Skills cover different tasks.

## Install a Skill

Install into the detected Agent target:

```sh
skilld install <skill>
```

Install into global Agent targets:

```sh
skilld install <skill> --global
```

Install this skilld-maintained Skill globally:

```sh
skilld install skilld --global
```

Use the source selector shown by `skilld search` when names are ambiguous.
If private repository access is required, run `skilld auth login`.
Do not print access tokens or copy them into project files.

After installation, report the installed Skill name and Agent target.

## View a Skill

```sh
skilld list
skilld view <skill>
```

Use `list` to show installed Skills.
Use `view` to show one Skill's source, version, files, and check results.

## Maintain installed Skills

```sh
skilld upgrade <skill>
skilld verify <skill>
skilld remove <skill>
```

Use `upgrade` to install a newer Artifact.
Use `verify` to check the installed bytes and source status.
Use `remove` only when the request names the Skill to remove.
