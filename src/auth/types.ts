/** Token payload returned by `POST /api/cli/oauth/token` and `…/refresh`. */
export interface TokenResponse {
  accessToken: string
  refreshToken?: string
  expiresAt: number
  login: string
  scopes?: string
}
