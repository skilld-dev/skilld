import { writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { skilldV1Protocol } from '../src/contract/index'
import { buildOpenApiDocument, serializeContractDocument } from '../src/contract/openapi'

/**
 * Writes the committed OpenAPI document. The site serves this file at
 * `/api/v1/openapi.json`, and a test fails when it drifts from the contract.
 */
const target = fileURLToPath(new URL('../generated/openapi.v1.json', import.meta.url))
await writeFile(target, serializeContractDocument(buildOpenApiDocument(skilldV1Protocol)), 'utf8')
