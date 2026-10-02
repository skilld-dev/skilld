# skilld-sdk

The typed TypeScript SDK and the OpenAPI document for the skilld API at `https://skilld.dev/api/v1`.

The skilld API reads the skilld registry and manages your account: likes, watches, collections, and the digest.
Every Skill in an answer carries its provenance: the Owner, the Repository, and the exact `SKILL.md`.

## Install

```sh
npm install skilld-sdk
```

The package is ESM only. It uses the runtime's global `fetch`, or a `fetch` you pass in.

## Quick start

```ts
import { createSkilldClient } from 'skilld-sdk'

// Public operations work without a token.
const skilld = createSkilldClient({ token: process.env.SKILLD_TOKEN })

const found = await skilld.skills.search({ query: { q: 'tailwind' } })
if (found._tag === 'Err')
  throw new Error(found.error._tag)

for (const item of found.value.items)
  console.log(`${item.source.owner}/${item.source.repository}/${item.name}`)

const skill = await skilld.skills.get({
  params: { owner: 'vercel-labs', repository: 'agent-skills', name: 'web-design-guidelines' },
})
if (skill._tag === 'Ok')
  console.log(skill.value.runCommand, skill.value.sourceUrl)
```

Each operation has a place on the SDK: `skilld.<namespace>.<key>(input)`.
The input holds up to three locations: `params` for the path, `query` for the query string, and `body` for the JSON body.
The SDK checks the input against the operation's schema before it sends anything.

## Results and failures

Every call returns a result. Check `_tag` before you read `value`.

```ts
type Result<TValue, TError>
  = | { _tag: 'Ok', value: TValue, requestId?: string }
    | { _tag: 'Err', error: TError }
```

Nothing throws for an expected failure.
`createSkilldClient` throws a `TypeError` only for bad options, such as a retry count out of range or a runtime with no `fetch`.

An `Err` holds one of four failures. Each has a `_tag` and the `operationId` of the call.

| `_tag` | When | What to do |
| --- | --- | --- |
| `RequestFailure` | The input failed the operation's schema, so the SDK sent nothing. `location` and `issues` say which input. | Fix the input. |
| `ApiFailure` | skilld.dev answered with a problem the operation declares. It carries `code`, `status`, `title`, `detail`, and `retryable`. | Read `code`. For `AUTH_REQUIRED`, send a skilld token. |
| `ContractFailure` | skilld.dev answered with something the contract does not allow. | Report it with `requestId`. |
| `TransportFailure` | No answer arrived. `reason` is `network`, `aborted`, or `credential`. `credential` means the token function threw. | Retry later if `retryable` is true. |

An `ApiFailure.code` is one of these:

| Code | Status | Retryable |
| --- | --- | --- |
| `INVALID_REQUEST` | 400 | no |
| `AUTH_REQUIRED` | 401 | no |
| `FORBIDDEN` | 403 | no |
| `NOT_FOUND` | 404 | no |
| `CONFLICT` | 409 | no |
| `RATE_LIMITED` | 429 | yes |
| `INTERNAL_ERROR` | 500 | no |
| `SERVICE_UNAVAILABLE` | 503 | yes |

Every operation can answer `INTERNAL_ERROR` and `SERVICE_UNAVAILABLE`. The OpenAPI document lists the other codes for each operation.

On the wire, every failure is RFC 9457 `application/problem+json` with exactly six fields: `type`, `title`, `status`, `detail`, `instance`, and `code`.
Every answer carries an `X-Request-Id` header. The SDK puts it on the result as `requestId`. Quote it when you report a problem.

## Authentication

Public operations need no credential. Account operations need a skilld token, or they answer `AUTH_REQUIRED`.

- To create a skilld token for a script, open [skilld.dev/me/cli-tokens/new](https://skilld.dev/me/cli-tokens/new).
- `skilld auth login` stores a skilld token for the skilld CLI in the operating system keychain. The SDK does not read it.
- To revoke a skilld token, open [skilld.dev/me/devices](https://skilld.dev/me/devices).

Pass the skilld token as a string, or as a function that returns one:

```ts
const skilld = createSkilldClient({
  token: async () => readTokenFromYourVault(),
})
```

The SDK calls a token function before each attempt, so a rotated token takes effect on the next call.
It sends the skilld token as `Authorization: Bearer <token>`.

A page served from skilld.dev can use the sign-in cookie instead of a skilld token.
Account operations send no CORS header, so the cookie works only on skilld.dev itself.

## Retries

The SDK retries only a call that is safe to send again:

- a query (every `GET`)
- a `PUT` or `DELETE` mutation, because each one names the end state

A `POST` or `PATCH` is never sent twice.

A safe call retries after a network failure, `RATE_LIMITED`, or `SERVICE_UNAVAILABLE`.
The delay doubles from 200 ms up to 5 s. If skilld.dev sends `Retry-After`, the SDK waits at least that long, up to 60 s.
The default is 3 attempts, the first one included.

```ts
const skilld = createSkilldClient({
  retry: { maxAttempts: 5, baseDelayMs: 500, maxDelayMs: 10_000 },
})
```

To turn retries off, set `retry: { maxAttempts: 1 }`.
To cancel a call, pass an `AbortSignal`: `skilld.skills.search({ query: { q: 'vue' } }, { signal })`.

## Other options

| Option | Default | Use |
| --- | --- | --- |
| `baseUrl` | `https://skilld.dev` | Point the SDK at a local or preview deployment. |
| `fetch` | `globalThis.fetch` | Supply your own `fetch`, for example in a test. |
| `headers` | none | Add headers to every request. |
| `credentials` | the `fetch` default | Passed to `fetch` as is. |

## The OpenAPI document

The OpenAPI 3.1 document describes every operation: its path, its input, its answer, its errors, and an example.

- skilld.dev serves it at [`https://skilld.dev/api/v1/openapi.json`](https://skilld.dev/api/v1/openapi.json).
- The package ships it as `skilld-sdk/openapi.json`.

```ts
import document from 'skilld-sdk/openapi.json' with { type: 'json' }
```

`skilld-sdk/contract` exports the operation descriptors and their [Zod](https://zod.dev) schemas.
`skilld.execute(operation, input)` calls an operation by its descriptor.

## Versioning

The API version is the URL major: `/api/v1`.

- Within v1, answers only gain fields. A field never changes its meaning, and a field never disappears.
- The SDK keeps unknown fields when it reads an answer, so a newer server never breaks an older SDK.
- A deprecated operation sends a `Deprecation` header. The OpenAPI document marks it, and names its replacement when one exists.
- A change that would break a caller ships under a new URL major, such as `/api/v2`.

One exception: the `skills.search` answer is frozen.
skilld 3.2.0 parses it with strict rules, so it never gains a field. Read new fields from `skills.get`.

The package version follows semver on its own, apart from the API version.

## Development

This package lives in the skilld.dev repository at `packages/sdk`, beside the server that serves the API.
One descriptor per operation in `src/contract` drives the OpenAPI document, the server's route checks, this SDK, and a route parity test.
ADR-0006 in that repository records the decisions.

```sh
pnpm --filter skilld-sdk generate   # rewrite generated/openapi.v1.json from the contract
pnpm --filter skilld-sdk test       # fails when the committed document drifts from the contract
pnpm --filter skilld-sdk typecheck
```

If you change a descriptor, run `generate` and commit the document with the change.

## License

MIT
