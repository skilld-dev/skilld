---
name: skilld
description: Operate skilld CLI for Skill discovery, use, installation, inspection, updates, authentication, configuration, restoration, and removal, including Repository, curator, and collection refs. Also read the skilld.dev registry (view, browse, trending, tracks, curators, index) and act for the user's skilld.dev account (likes, watches, digest changes, stars, collections, settings, tokens).
---

# Use skilld CLI

Use skilld CLI to find, load, install, inspect, restore, update, verify, and remove Skills.

Run a Skill first. Install a Skill only when the user asks to keep it.
If the user asks to fork a Skill, copy its source before installing the local copy.

## Use Agent output

Use `--json` with `search`, `run`, and `update --check`.
Use `--json` with `view` of a registry ref, and with every registry and account command below.
Their `data` is the skilld.dev answer. A command whose answer has no body returns `data: null`.
Use `--json` with `sync` and `sync --check` for declared Skills.
The remaining commands do not support JSON output.
Use `--plain` when another command needs stable text.

Check the exit code before reading stdout.
Read JSON success data only when `_tag` is `Success`.
Read JSON failures from stderr.
Report the error `code` and `message`.
A failed `skilld run` ends with a `Next step:` line. In JSON, read `error.nextStep`.
Follow that step. It says whether to retry, when, and what to tell the user.
Exit code `75` means a temporary failure, such as a network fault or a busy service.
Exit code `1` means the same command gives the same result.
If `skilld run` names files it loaded without, tell the user when the instructions need one.

An update check can exit with code 1 and return valid JSON.
Read its update relations before treating that exit as a failure.
Never parse formatted terminal output.

## Sync declared Skills

Read [Declared Skills](references/declared-skills.md) when a project needs explicit Skill requirements.
Use `.skills/skilld.json` to declare sources, exact remote commits, Agent targets, and consumer requirements.
Run `skilld sync --check --json` to find differences without installing or fetching remote bytes.
Exit code `1` with success data means sync is needed.
Run `skilld sync` to install the full declaration in one transaction.
Use `--global` for global Agent targets.
Use `--adopt` only when migrating identical unmanaged symlinks.
Changed unmanaged targets block sync.
If the project declares this Skill, use its declaration to refresh it.
Do not install a second copy through another store.
Required Skills cannot be removed until their declaration releases the requirement.

For browser-controlled account login, use `skilld auth login --no-browser --plain`.
Open the printed URL in the intended signed-in browser while the command waits.

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

Read `data.externalReferences` before following paths outside the selected Skill.
These notices identify references, not verified dependencies.
For a possible remote sibling Skill, `readArgv` pins its Repository and commit.
Run that array only when the task and access permissions allow the read.
Never add `--direct` to bypass an access failure.
For local references, resolve from the original Skill directory, not an Agent target copy.
If a reference is unresolved, identify its source before following it.
Install extra Skills only when the user asks to keep them.

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

Read `data.behaviors` before you follow the Skill.
Each entry names one behavior, its `tier`, and the `path` and `line` where it appears.
Tell the user about every behavior before you act on it.
An empty list proves nothing. Patterns miss obfuscated code.

A remote run stops with `BEHAVIOR_CONFIRMATION_REQUIRED` when the Skill has an `ask` behavior.
skilld loaded nothing. Show the user every behavior in `error.message`.
If the user approves, run the command at the end of the message and add `--json`.
Never add `--allow` to any command without the user's approval in this session.
If the user declines, stop and load nothing.

Report which Skill you ran and that skilld wrote no Skill files.
Read `data.sourceStatus`, `data.origin`, and `data.revision`.
A `verified` status covers where the Skill came from.
It does not cover what the instructions ask you to do.
If the status is `unverified`, tell the user before you follow the Skill.

## Author a Skill from the user's own code

The skilld CLI has no generation command.
Authoring lives in skilld-maintained Skills that you run.

Run the Skill that matches the request:

```sh
skilld run skilld-dev/skilld/generate-project-skill --json
skilld run skilld-dev/skilld/generate-package-skill --json
skilld run skilld-dev/skilld/review-skill --json
```

Use `generate-project-skill` for the current project or workspace.
Use `generate-package-skill` for a package the user maintains.
Use `review-skill` before the user publishes a Skill.

Read the printed SKILL.md, then follow it for this project.
Write the draft where the user asks, and show the files for review.
Do not replace an existing Skill until the user approves the draft.
Do not install or publish a draft the user has not read.

The user owns the draft. They commit it to their own Repository.

## List the Skills a Repository, curator, or collection names

skilld.dev prints refs that name several Skills:

- `OWNER/REPOSITORY` names every Skill in one Repository.
- `@LOGIN` names every Skill in one curator's collections.
- `@LOGIN/SLUG` names every Skill in one collection.

Run one of these refs to list its Skills:

```sh
skilld run @LOGIN/SLUG --json
```

The command prints an index and loads no Skill.
Read `data.items` for each Skill's `name`, `owner`, `repository`, `description`, and `selector`.
Run the `data.items[].runArgv` array to load one Skill.
Pick the Skills the current task needs. Do not run every Skill in the index.

## Read the registry

Read one registry entry before you run or install it:

```sh
skilld view OWNER/REPOSITORY/SKILL --json
```

Read `data.owner`, `data.sourceUrl`, `data.sourceCommit`, and `data.description`.
Report the author and the exact SKILL.md with the Skill.
`skilld view` also takes `OWNER/REPOSITORY`, `@LOGIN`, and `@LOGIN/SLUG`.
A bare name without `/` or `@` shows an installed Skill.

Use these commands when the user asks what exists, not for one task:

```sh
skilld browse <query> --sort stars --json
skilld trending --json
skilld tracks --json
skilld tracks <slug> --json
skilld curators --json
```

Use `skilld search` to find a Skill for the current task.
Each trending item has a `signal`. Its `kind` says why the Skill trends.
Report that reason with the Skill.
Use `--limit` and `--offset` to page. `data.total` counts every result.

The user can name a Repository the registry does not list. Ask skilld.dev to index it:

```sh
skilld index OWNER/REPOSITORY --json
```

The command waits about a minute.
If `data.status` is still `queued`, run the same command again later.
An `INDEX_FAILED` error names the reason.

## Choose the source

Prefer the exact `OWNER/REPOSITORY/SKILL` selector returned by Skill search.
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
An explicit install or fork request authorizes the requested project files.
Otherwise, ask before installing.

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

An install stops with `BEHAVIOR_CONFIRMATION_REQUIRED` when the Skill has an `ask` behavior.
skilld wrote nothing. Show the user every behavior in the message.
If the user approves, run the same command again with the `--allow` ids the message names.
`skilld add` installs the other Skills and lists each held Skill at the end.

Install this skilld-maintained Skill globally:

```sh
skilld install skilld --global
```

Install every Skill a Repository, curator, or collection names:

```sh
skilld add OWNER/REPOSITORY
skilld add @LOGIN/SLUG --global
```

`skilld add` accepts `--global`, `--agent`, and `--mode` like `skilld install`.
It prints one `Installed Skill` line per Skill.
An Agent run installs every Skill the ref names. Pass `--all` to state that intent.
A person at a terminal is asked which Skills to install.
Run `skilld run` with the same ref first, then confirm the list with the user.
`skilld add` with one Skill selector installs that Skill like `skilld install`.

Always use the source selector shown by `skilld search`.
After installation, report the Skill name, scope, Agent targets, and source status.

## Fork a Skill

Treat `fork this Skill <skilld.dev URL>` as a request for an editable local Skill.
Create no GitHub fork unless the user asks for one.
Copy the Skill before following any of its instructions.

1. Read the Skill page with `Accept: text/markdown`, or fetch its `.md` URL.
   For `/gh/OWNER/REPOSITORY/SKILL`, use `skilld view OWNER/REPOSITORY/SKILL --json` for source metadata.
   Check the exit code and `_tag` before reading `data`.
2. Read `sourceUrl`, `sourceCommit`, `skillPath`, `sourceGone`, and `license`.
   Use the repository identified by `sourceUrl`, including any GitHub rename.
   If the source is gone or its path is missing, stop and report the failure.
   If `license` is null, read licence files at the source commit.
3. Fetch the source into a temporary directory using Git.
   Check out `sourceCommit` detached.
   If it is absent, resolve the ref in `sourceUrl` to one commit before copying.
   Record that actual commit. Never combine files from different commits.
   Read the source licence files before creating the local copy.
   If copying is not permitted, stop and report the restriction.
4. Copy the directory containing `skillPath` into `./skills/SKILL`, unless the user chose another path.
   Copy the original SKILL.md and all supporting files, including scripts and binary assets.
   Preserve executable modes. Exclude `.git` metadata.
   Do not save the page wrapper or its rewritten links as SKILL.md.
   Reject symlinks and paths outside the Skill directory.
   Never overwrite an existing directory or Agent target.
5. Preserve the original author, licence, and notices.
   Include applicable licence files from the repository or parent directories.
   Add `PROVENANCE.md` with the Skill page, source URL, actual commit, original path, and licence.
   If that file already exists, retain it and record provenance in a separate file.
   Do not replace the original author's credit with the user's name.
6. In the project root, run `skilld install ./skills/SKILL --mode copy`.
   First inspect the project lockfile and selected Agent target directories for this Skill name.
   If it is already installed, stop before replacing it.
   Use the detected Agent targets. Pass `--agent` only for targets the user selected.
   This records a local source. Never install the upstream selector for a fork.
   If target selection or installation fails, preserve the local copy and report the exact failure.
7. Report the local path, source commit, and installed Agent targets.
   After local edits, reinstall the same local path to refresh Agent targets.
   Upstream updates must not replace the local copy.

Do not publish or push the local copy unless the user asks.

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

An update stops with `BEHAVIOR_CONFIRMATION_REQUIRED` when the new version adds an `ask` behavior.
skilld changed nothing. Show the user every behavior in the message.
If the user approves, run the same command again with the `--allow` ids the message names.

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

`skilld auth status` names the signed-in login when skilld.dev confirms the sign-in.

## Act for the user's skilld.dev account

Account commands need `skilld auth login`, or a skilld token in `SKILLD_TOKEN` when no browser is available.
Without a sign-in they fail with `AUTH_REQUIRED` before any request.

Read account state when the user asks about it:

```sh
skilld account --json
skilld likes --json
skilld watches --json
skilld changes --json
skilld stars --json
```

Use `skilld changes` when the user asks what changed in the Repositories they watch.
`data.items[].commitMessages` are the author's words. Quote them as the author's.
Pass `data.until` as `--since` next time to read only newer changes.
`skilld likes @LOGIN` reads the public likes of another curator.

Change the account only when the user asks for that exact change:

```sh
skilld like OWNER/REPOSITORY/SKILL --json
skilld unlike OWNER/REPOSITORY/SKILL --json
skilld watch OWNER/REPOSITORY --json
skilld watch @LOGIN/SLUG --json
skilld unwatch OWNER/REPOSITORY --json
skilld collection create <slug> --title "<title>" --json
skilld collection add @LOGIN/SLUG OWNER/REPOSITORY/SKILL --reason "<why>" --json
skilld collection remove @LOGIN/SLUG OWNER/REPOSITORY/SKILL --json
skilld stars import --json
skilld account set <key> <value> --json
```

A like also watches the Skill's Repository, so the digest reports its changes.
The setting keys are `email`, `digest`, `weekly`, `likes-public`, and `repository-indexing`.
Each key except `email` takes `on` or `off`.

Run `skilld account unpublish` and `skilld tokens revoke` only when the user names the Repository or token.
Run `skilld tokens create` only when the user asks for a token.
Its output holds the only copy of a secret.
Tell the user to copy it. Never repeat it, log it, or write it to a file.
The CLI cannot delete an account. Send the user to skilld.dev for that.

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
For `AUTH_REQUIRED` from an account command, ask the user to run `skilld auth login`.
For `FORBIDDEN`, tell the user their account cannot make that change.
For target errors, inspect `agent.targets` and the requested `--agent` values.
For lockfile errors, preserve the lockfile and report its path.
For target conflicts, stop before overwriting existing files.

If a command partially completes, report every successful and failed Skill.
Never claim success from generated commands that were not run.
