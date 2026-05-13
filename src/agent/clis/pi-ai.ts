/**
 * pi-ai adapter, direct LLM API calls via @earendil-works/pi-ai.
 *
 * Optional alternative to CLI spawning. Supports env-var API keys
 * (ANTHROPIC_API_KEY, GEMINI_API_KEY, OPENAI_API_KEY, etc.).
 *
 * OAuth providers are blocked by default. Consumer subscription OAuth
 * impersonates official CLI clients and violates provider ToS, risking
 * account bans. Use API keys or native CLI tools (claude, gemini, codex).
 *
 * Implementation is split across sibling files; this module re-exports the
 * public surface so callers (commands/, llm-enhancer) only need one import.
 */

export type { LoginCallbacks } from './pi-ai-auth.ts'
export { getOAuthProviderList, loginOAuthProvider, logoutOAuthProvider } from './pi-ai-auth.ts'
export type { PiAiModelInfo } from './pi-ai-models.ts'
export { getAvailablePiAiModels, isPiAiModel, parsePiAiModelId } from './pi-ai-models.ts'
export type { PiAiSectionOptions, PiAiSectionResult } from './pi-ai-runner.ts'
export { optimizeSectionPiAi } from './pi-ai-runner.ts'
