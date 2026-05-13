# skilld-protocol

Wire shapes and constants shared between the [skilld CLI](https://github.com/skilld-dev/skilld) and [skilld.dev](https://skilld.dev). The single source of truth for everything that crosses that boundary: telemetry, audit, auth, device flow, collection manifests.

## Install

```sh
pnpm add skilld-protocol
```

ESM-only. Node ≥18. One peer-free dep: `zod` v4.

## Subpaths

- `skilld-protocol/wire` — every endpoint shape as a zod schema (suffix `Schema`) and the matching inferred TS type (no suffix). Use `import { FooSchema }` for runtime validation; `import type { Foo }` for the type.
- `skilld-protocol/constants` — readonly tuples backing the closed enums plus their inferred unions.
- `skilld-protocol/test-fixtures` — canonical payloads each consumer round-trips through their schema on CI.

## Repo

This package lives inside the [skilld CLI](https://github.com/skilld-dev/skilld) monorepo at `packages/protocol`. The CLI consumes it as a workspace dep; skilld.dev consumes the published npm version.
