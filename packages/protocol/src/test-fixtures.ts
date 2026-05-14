/**
 * Canonical payloads each consumer round-trips through the wire schemas on CI.
 * A breaking schema change invalidates a fixture → both consumer test suites
 * go red on the bump. Drift detector at zero conditional-logic cost.
 */

import type {
  AuditEntry,
  ChangeEntry,
  CliEventInput,
  CollectionManifest,
  CollectionSummary,
  DevicePollInput,
  DevicePollResponse,
  DeviceStartInput,
  DeviceStartResponse,
  DigestResponse,
  OauthRefreshInput,
  OauthTokenInput,
  OidcExchangeInput,
  SkillDetailResponse,
  SkillLiveResponse,
  SkillsResolveInput,
  SkillsResolveResponse,
  TokenResponse,
} from './wire.ts'

export const fixtures = {
  audit: {
    skillLivePass: {
      id: 'antfu/skills/vue',
      installs: 1234,
      formatted: '1.2k',
      audits: [
        { provider: 'skills.sh', slug: 'static', status: 'pass' },
        { provider: 'skills.sh', slug: 'license', status: 'pass' },
      ],
      source: 'skills.sh',
      fetchedAt: '2026-05-13T00:00:00.000Z',
    },
    skillLiveWarn: {
      id: 'antfu/skills/motion-v',
      installs: 42,
      formatted: '42',
      audits: [
        { provider: 'skills.sh', slug: 'static', status: 'pass' },
        { provider: 'skills.sh', slug: 'deps', status: 'warn', summary: 'wildcard import', riskLevel: 'medium', categories: ['imports'] },
      ],
      source: 'skills.sh',
      fetchedAt: '2026-05-13T00:00:00.000Z',
    },
    entryFail: {
      provider: 'skills.sh',
      slug: 'static',
      status: 'fail',
      summary: 'detected eval()',
      auditedAt: '2026-05-12T18:00:00.000Z',
    },
  },
  auth: {
    tokenResponse: {
      accessToken: 'eyJhbGc...stub',
      refreshToken: 'r-32-bytes-base64url',
      expiresAt: 1_715_600_000,
      login: 'harlanzw',
      scopes: 'cli',
    },
    tokenResponseOidc: {
      accessToken: 'eyJhbGc...oidc',
      expiresAt: 1_715_600_000,
      login: 'harlanzw',
    },
    oauthTokenInput: {
      code: 'auth-code-1234567890abcdef',
      code_verifier: 'verifier-32-bytes-or-more-padding-here',
      redirect_uri: 'http://127.0.0.1:50123/',
    },
    oauthRefreshInput: {
      refresh_token: 'r-32-bytes-base64url',
    },
    oidcExchangeInput: {
      id_token: `${'a'.repeat(64)}.payload.signature`,
    },
  },
  device: {
    startInput: {
      cli_version: '2.0.0',
      machine_hint: 'darwin-arm64',
    },
    pollInput: {
      device_code: 'dc-128-bytes-base64url-string',
    },
    startResponse: {
      device_code: 'dc-128-bytes',
      user_code: 'WDJB-MJHT',
      verification_uri: 'https://skilld.dev/cli/authorize',
      interval: 5,
      expires_in: 600,
    },
    pollPending: { status: 'pending' },
    pollAuthorized: {
      status: 'authorized',
      tokens: {
        accessToken: 'eyJhbGc...stub',
        refreshToken: 'r-32-bytes-base64url',
        expiresAt: 1_715_600_000,
        login: 'harlanzw',
      },
    },
  },
  telemetry: {
    installEvent: {
      event: 'install',
      surface: 'cli:add',
      sourceKind: 'npm',
      slug: 'vue',
      cliVersion: '2.0.0',
      agent: 'claude-code',
    },
    auditFailEvent: {
      event: 'audit-fail',
      surface: 'cli:pull',
      sourceKind: 'gh',
      slug: 'antfu/skills',
      cliVersion: '2.0.0',
    },
    authFlowEvent: {
      event: 'auth-flow',
      surface: 'cli:auth',
      cliVersion: '2.0.0',
      flow: 'pkce',
    },
  },
  collections: {
    manifest: {
      name: 'My Vue stack',
      preamble: 'Tools I reach for on every Vue project.',
      items: [
        { kind: 'npm', package: 'vue' },
        { kind: 'npm', package: 'motion-v' },
        { kind: 'gh', owner: 'antfu', repo: 'skills' },
      ],
    },
    summary: {
      slug: 'vue-stack',
      name: 'My Vue stack',
      itemCount: 3,
    },
    change: {
      repo: 'antfu/skills',
      skill: 'vue',
      at: '2026-05-13T00:00:00.000Z',
      summary: 'Added Pinia composables.',
    },
    digestResponse: {
      user: { id: 2, login: 'harlan-zw' },
      windowStart: 1_715_500_000,
      windowEnd: 1_715_600_000,
      entries: [
        { repo: 'antfu/skills', skill: 'vue', at: '2026-05-13T00:00:00.000Z', summary: 'Added Pinia composables.' },
      ],
    },
  },
  skills: {
    resolveInput: {
      items: [
        { packageName: 'vue' },
        { packageName: 'motion-v', owner: 'antfu' },
      ],
    },
    resolveResponse: {
      'vue': { owner: 'antfu', repo: 'skills', official: true },
      'motion-v': { owner: 'antfu', repo: 'skills', official: false },
    },
    detail: {
      owner: 'antfu',
      repo: 'skills',
      name: 'vue',
      displayName: 'Vue',
      installs: 1234,
      branch: 'main',
      skillPath: 'vue/SKILL.md',
      raw: '# Vue\n\nUse <script setup>.',
      pushedAt: '2026-05-13T00:00:00.000Z',
    },
  },
} satisfies {
  audit: { skillLivePass: SkillLiveResponse, skillLiveWarn: SkillLiveResponse, entryFail: AuditEntry }
  auth: {
    tokenResponse: TokenResponse
    tokenResponseOidc: TokenResponse
    oauthTokenInput: OauthTokenInput
    oauthRefreshInput: OauthRefreshInput
    oidcExchangeInput: OidcExchangeInput
  }
  device: {
    startInput: DeviceStartInput
    pollInput: DevicePollInput
    startResponse: DeviceStartResponse
    pollPending: DevicePollResponse
    pollAuthorized: DevicePollResponse
  }
  telemetry: { installEvent: CliEventInput, auditFailEvent: CliEventInput, authFlowEvent: CliEventInput }
  collections: {
    manifest: CollectionManifest
    summary: CollectionSummary
    change: ChangeEntry
    digestResponse: DigestResponse
  }
  skills: {
    resolveInput: SkillsResolveInput
    resolveResponse: SkillsResolveResponse
    detail: SkillDetailResponse
  }
}
