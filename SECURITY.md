# Security

## Report a vulnerability

Report a vulnerability in the skilld CLI, the Harness, or the skilld.dev API in private.

- Use [GitHub private vulnerability reporting](https://github.com/skilld-dev/skilld/security/advisories/new) on `skilld-dev/skilld`.
- Or email harlan@harlanzw.com.

Do not open a public issue for a vulnerability.
Include the CLI version, the command you ran, and the steps to reproduce.

To report a Skill that misleads or harms users, email the same address with the Skill selector.

## What skilld checks

The skilld CLI verifies remote Skills before it installs them.
For every Artifact from skilld.dev, the CLI checks:

- the exact statement bytes of the Artifact attestation
- the attestation signature against a trusted root key compiled into the CLI
- the SHA-256 digest of the Artifact contents
- the archive structure, including a `SKILL.md` file at the Skill root with a `name`

If no trusted root key exists, the CLI fails with `TRUSTED_ROOT_UNCONFIGURED`.
If any check fails, the CLI installs nothing.

The CLI stores account credentials in the operating system keychain.
It never writes tokens to environment variables or plain text files.

## What the source statuses mean

The lockfile records one source status for each installed Skill.

- `verified`: the CLI checked a skilld.dev Artifact and its attestation. The Skill bytes match one exact commit in the source Repository.
- `local`: the Skill came from a local directory or a bundled skilld-maintained Skill. The CLI checked no remote source.
- `unverified`: direct mode fetched the Skill from public GitHub without the skilld.dev API. The CLI checked no attestation.

## What skilld never claims

A source status describes provenance.
It states where a Skill came from and that its bytes did not change in transit.

skilld never claims that a Skill is safe.
skilld does not review the instructions inside a Skill.
Check results from third parties describe their own findings, not an endorsement by skilld.

Read a Skill before your Agent follows it.
`skilld run` prints `SKILL.md` so you can read it first.
