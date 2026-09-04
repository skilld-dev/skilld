# Migrate from skilld v2 to v3

v3 replaces the JavaScript CLI with a native Rust CLI.
It also moves Skill generation into visible Skills and `skilld-harness`.

v3 does not import v2 configuration, credentials, or lockfiles.
Complete these steps in order.

## 1. Record and back up v2 state

Run these commands before replacing v2:

```sh
skilld --version
skilld list > skilld-v2-skills.txt
```

Copy `~/.skilld/config.yaml` to a backup when it exists.

Move each project `.skills` directory to an unused backup path.
For example:

```sh
mv .skills .skills-v2-backup
```

The v2 and v3 lockfiles share a name but use different schemas.
v3 will reject a v2 `.skills/skilld-lock.yaml`.

Do not remove Agent target Skills yet.
They are your rollback copy.

## 2. Install and configure v3

Install the v3 CLI:

```sh
npm install --global skilld@3.0.0-beta.1
skilld --version
```

Map the old `agent` value to `agent.targets`:

```sh
skilld config set agent.targets codex
skilld config set install.mode copy
skilld config list
```

Use a comma separated value when you need several Agent targets.

v3 writes `~/.skilld/config.json`.
It leaves the v2 `~/.skilld/config.yaml` unchanged.

The v2 model, embedding, generation, cache, and feature settings have no v3 equivalent.

## 3. Authenticate again when required

v3 stores account credentials in the operating system keychain.
It does not read the v2 `~/.skilld/auth.json` file.

Run:

```sh
skilld auth login
skilld auth status
```

You only need an account for private repository access.

## 4. Run the skilld Skill

Ask your Agent to load search, run, and install guidance for the current session:

```sh
skilld run skilld
```

Install it globally only when you want your Agent to keep it across sessions:

```sh
skilld install skilld --global --agent codex
```

Change `codex` to the required Agent target.
You may omit `--agent` after configuring or detecting a target.

## 5. Replace project Skills

v3 stores canonical project Skills under `.skills/<name>`.
It writes project state to `.skills/skilld-lock.yaml`.

Search first, then run the returned selector:

```sh
skilld search vue
skilld run skilld:skilld-dev/skills/vue
```

Use the exact selector printed by your search result.

`skilld run` loads a transient Skill for the current Agent session.
It writes no project files or lockfile state.

Install the Skill only when you want to keep it:

```sh
skilld install skilld:skilld-dev/skills/vue
```

An install writes project files and records lockfile state.

An existing v2 Skill in an Agent target is unmanaged v3 state.
v3 returns `TARGET_CONFLICT` instead of replacing it.

If a conflict occurs:

1. Review the existing Agent target Skill.
2. Move that Skill directory to a backup path.
3. Run the v3 install again.
4. Compare the old and new instructions before deleting the backup.

Run `skilld install` without a selector to restore v3 lockfile state.
It cannot restore a v2 lockfile.

## Command replacements

| v2 command | v3 replacement |
| --- | --- |
| `skilld add <source>` | Run `skilld search`, then `skilld run <selector>` to use it once, or `skilld install <selector>` to keep it |
| `skilld add gh:OWNER/REPOSITORY`, `skilld add @LOGIN`, `skilld add @LOGIN/SLUG` | Unchanged. `skilld run` with the same ref lists the Skills first |
| `skilld add npm:PACKAGE` | Not supported. Run `skilld search <package>`, then use the selector |
| `skilld update [name]` | `skilld update [name]` |
| `skilld info` | `skilld list`, then `skilld view <name>` |
| `skilld login` | `skilld auth login` |
| `skilld whoami` | `skilld auth status` |
| `skilld logout` | `skilld auth logout` |
| `skilld prepare` | Run `skilld install` explicitly in CI |
| `skilld author package` | Run `generate-package-skill` with an Agent, or use Harness `PackageSkill` |
| `skilld author validate` | Run `review-skill` with an Agent, or use Harness `ReviewSkill` |
| `skilld author assemble` | Apply approved Agent changes, or let Harness promote checked output |
| `skilld author eject` | Copy the reviewed canonical Skill directory |
| `skilld author publish` | Commit the Skill to a GitHub repository |

The v2 `watch`, `unwatch`, `cache`, `changes`, `setup`, `uninstall`, and `pull` commands are removed.
v3 has no one for one replacements for them.

## Choose direct Agent use or Harness

Use the visible skilld-maintained Skills for an interactive Agent run.
The Agent shows the proposed files.
You review and approve any replacement.

Use `skilld-harness` in CI or an application.
Harness enforces output limits, structure checks, and atomic promotion.

An Agent run does not claim that Harness checks passed.
CI should remain strict and use Harness.

`skilld install --direct` and `skilld run --direct` serve another purpose.
They read a public GitHub repository without skilld.dev Artifact delivery.
Explicit GitHub selectors use Artifact delivery unless you add `--direct`.
A direct install records the `unverified` source status.
A direct run reports the same status but writes no files.
Direct mode never handles private repositories.

## Release requirements

Set these GitHub Actions repository variables before creating a v3 tag:

- `SKILLD_ROOT_KEY_ID`
- `SKILLD_ROOT_PUBLIC_KEY`

The public key must be the pinned Ed25519 root public key in canonical base64url form.
Keep the root private key outside this repository and GitHub Actions.

The release tag must match the root `package.json` version exactly.
The CLI, native packages, and Harness use that same version.
The protocol package has an independent version.

Release reruns skip an exact package version that npm already contains.
A registry error or mismatched response stops the release.

WASIp2 remains unpublished.

## Roll back

Run `skilld auth logout` before downgrading when you used private repository access.

Install v2 again:

```sh
npm install --global skilld@2.3.0
```

This command only restores the v2 executable.
It does not convert v3 state.

Restore the backed up v2 `.skills` directory and Agent target Skills yourself.
v2 cannot read the v3 lockfile or `config.json`.
v3 cannot read the v2 lockfile or `config.yaml`.

npm package versions are immutable.
Publish a new version when a released Artifact or package is wrong.
