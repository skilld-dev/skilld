# Account and configuration

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

