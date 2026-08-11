const HAS_SCHEME_RE = /^https?:\/\//
const TRAILING_SLASH_RE = /\/$/

/**
 * Base URL for the local Ollama daemon, normalised so callers can append paths.
 *
 * Lives in `core/` rather than `agent/clis/ollama.ts` so the search worker can
 * reach it without pulling in the agent registry.
 */
export function ollamaHost(): string {
  const raw = process.env.OLLAMA_HOST || 'http://localhost:11434'
  const withScheme = HAS_SCHEME_RE.test(raw) ? raw : `http://${raw}`
  return withScheme.replace(TRAILING_SLASH_RE, '')
}
