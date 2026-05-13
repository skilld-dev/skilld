/**
 * Auth endpoint wire shapes.
 *
 * `TokenResponse` is returned by `POST /api/cli/oauth/token` and
 * `POST /api/cli/oauth/refresh`. `AuthSession` is the in-memory shape the CLI
 * builds from a token response to carry through API calls — its `accessToken`
 * is sent as `Authorization: Bearer …` against authed endpoints.
 */

import { z } from 'zod'

/**
 * Inputs to `POST /api/cli/oauth/token` (PKCE exchange),
 * `POST /api/cli/oauth/refresh`, and `POST /api/cli/oidc/exchange`.
 *
 * Mirror the snake_case field names the OAuth-style endpoints expect on the
 * wire. The CLI builds the body shape from these schemas; the server parses
 * the request through the matching schema before doing work.
 */

export const OauthTokenInputSchema = z.object({
  code: z.string().min(16),
  code_verifier: z.string().min(32).max(256),
  redirect_uri: z.string().url(),
})

export const OauthRefreshInputSchema = z.object({
  refresh_token: z.string().min(16),
})

export const OidcExchangeInputSchema = z.object({
  id_token: z.string().min(64),
})

export const TokenResponseSchema = z.object({
  accessToken: z.string(),
  refreshToken: z.string().optional(),
  expiresAt: z.number(),
  login: z.string(),
  scopes: z.string().optional(),
})

export const AuthSessionSchema = z.object({
  accessToken: z.string(),
  refreshToken: z.string().optional(),
  login: z.string(),
  expiresAt: z.number().optional(),
  host: z.string().optional(),
})

export type OauthTokenInput = z.infer<typeof OauthTokenInputSchema>
export type OauthRefreshInput = z.infer<typeof OauthRefreshInputSchema>
export type OidcExchangeInput = z.infer<typeof OidcExchangeInputSchema>
export type TokenResponse = z.infer<typeof TokenResponseSchema>
export type AuthSession = z.infer<typeof AuthSessionSchema>
