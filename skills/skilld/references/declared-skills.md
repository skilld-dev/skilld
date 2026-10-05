# Declared Skills

Use `.skills/skilld.json` when a project needs Skills on explicit Agent targets.
Run `skilld sync` to install the declaration.
Run `skilld sync --check --json` to check without installing or fetching remote bytes.
Exit code `1` with success data means the declaration needs sync.

```json
{
  "version": 1,
  "name": "my-project",
  "agents": ["codex", "claude-code", "opencode"],
  "mode": "symlink",
  "skills": {
    "write-human": {
      "source": "github:owner/repository/skills/write-human#commit:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    },
    "local-review": { "source": "../skills/local-review" }
  },
  "requires": { "pr": ["write-human", "local-review"] }
}
```

Replace the example commit with the exact source commit.
Remote sources use verified hosted Artifact delivery.
Private sources need a skilld account and access through the skilld GitHub App.
There is no direct GitHub fallback.

Local sources resolve relative to the declaration file.
The source directory must match the declared Skill name.
Sync copies local bytes into the managed store.
After a local edit, run sync again.
Symlink mode links each Agent target to that managed copy.

`requires` names each consumer and its required Skills.
Consumers can come from a separate Plugin.
Every required Skill must have a source in this declaration.
The lockfile records these requirements.
Removal refuses a required Skill until its consumer declaration releases it through sync.

Sync prepares every source before changing Agent targets.
One transaction writes installed Skills, targets, and requirements.
Preparation failures and target conflicts leave existing targets unchanged.
Repeated sync uses installed bytes when a remote source still matches its exact commit.
Sync preserves installed Skills omitted from the declaration.
Remove those Skills explicitly after releasing their requirements.

Use `--manifest PATH` to select another declaration.
Use `--global` for global Agent targets.
Use `SKILLD_DATA_DIR` to select a separate managed store.
This setting does not change global Agent target paths.

Existing unmanaged targets block sync.
Use `--adopt` to take ownership of an unmanaged symlink with identical Skill bytes.
Different bytes, directories, and broken links still block sync.
Sync preserves the original source directory.

Use a CLI containing `sync` to read lockfiles with recorded requirements.
Older CLIs reject those fields.
Keep legacy v2 stores separate; sync does not migrate their lockfiles.

## Account login from an Agent

Run `skilld auth login --no-browser --plain`.
The CLI prints an authorization URL and waits for its loopback callback.
Open that URL in the intended signed-in browser.
The CLI stores the resulting credential through its normal credential store.
Never copy browser cookies or tokens into a declaration.
