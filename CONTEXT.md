# skilld domain and architecture

Use the terms in `GLOSSARY.md` exactly.

## Product surfaces

- The skilld CLI is Rust.
- skilld-maintained Skills support direct Agent generation, review, search, and install.
- Harness is the JavaScript `@skilld/harness` package.
- `skilld.dev` resolves Repositories into Artifacts with attestations.

The skilld CLI and Harness do not import or execute each other.

skilld-maintained Skills are the only source for judgment instructions.

Direct runs remain user reviewed.

Harness runs apply deterministic checks before atomic promotion.

## Trust

- `verified` records an Artifact with a valid attestation.
- `local` records content read from the local filesystem.
- `unverified` records an explicit direct remote install.

Artifact attestations contain check results.

They never claim that a Skill is safe.

The skilld CLI never falls back to direct remote access automatically.

## Runtime boundaries

- Rust owns every skilld CLI command and decision.
- Native and WASIp2 adapters call one Rust command module.
- JavaScript selects a native package or hosts WASIp2 interfaces.
- Harness remains JavaScript and composes the AI SDK Harness.
- OpenAPI 3.1 is the cross-language protocol source.

## Filesystem state

- Project Skills live under `.skills/<name>`.
- Project state lives at `.skills/skilld-lock.yaml`.
- Global Skills live under the configured skilld data directory.
- Agent directories receive links or copies from canonical Skills.
- Installation promotes the canonical Skill before writing state.
- Interrupted changes recover from a transaction journal.
