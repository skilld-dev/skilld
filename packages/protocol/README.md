# skilld-protocol

Wire shapes and constants shared between the [skilld CLI](https://github.com/skilld-dev/skilld) and [skilld.dev](https://skilld.dev).
The package is the single source of truth for everything that crosses that boundary.

## Install

```sh
pnpm add skilld-protocol
```

ESM only. Node 22 or newer. One dependency: `zod` v4.

## Subpaths

- `skilld-protocol/v1`: TypeScript types generated from `openapi/skilld-v1.yaml`. This is the Artifact delivery contract the v3 CLI uses: source resolution, Skill search, Artifact descriptors, attestations, check results, grants, trusted roots, and problems.
- `skilld-protocol/openapi/skilld-v1.yaml`: the OpenAPI 3.1 document itself.
- `skilld-protocol/wire`: zod schemas (suffix `Schema`) and inferred types (no suffix) for the endpoints skilld.dev still serves to older clients. Use `import { FooSchema }` for runtime validation and `import type { Foo }` for the type.
- `skilld-protocol/constants`: readonly tuples behind the closed enums, plus their inferred unions.
- `skilld-protocol/test-fixtures`: canonical payloads that each consumer round-trips through its schema on CI.

## Repository

This package lives in the [skilld CLI](https://github.com/skilld-dev/skilld) monorepo at `packages/protocol`.
The Rust crates mirror the v1 OpenAPI document. skilld.dev consumes the published npm version.
