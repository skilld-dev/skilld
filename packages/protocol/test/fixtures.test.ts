/**
 * Every fixture must round-trip through its wire schema. This is the contract
 * both consumers replay in their own CI; locally we enforce it once here.
 */

import { describe, expect, it } from 'vitest'
import { fixtures } from '../src/test-fixtures.ts'
import {
  AuditEntrySchema,
  ChangeEntrySchema,
  CliEventInputSchema,
  CollectionManifestSchema,
  CollectionSummarySchema,
  DevicePollInputSchema,
  DevicePollResponseSchema,
  DeviceStartInputSchema,
  DeviceStartResponseSchema,
  OauthRefreshInputSchema,
  OauthTokenInputSchema,
  OidcExchangeInputSchema,
  SkillDetailResponseSchema,
  SkillLiveResponseSchema,
  SkillsResolveInputSchema,
  SkillsResolveResponseSchema,
  TokenResponseSchema,
} from '../src/wire.ts'

describe('fixture round-trip', () => {
  it.each([
    ['audit.skillLivePass', SkillLiveResponseSchema, fixtures.audit.skillLivePass],
    ['audit.skillLiveWarn', SkillLiveResponseSchema, fixtures.audit.skillLiveWarn],
    ['audit.entryFail', AuditEntrySchema, fixtures.audit.entryFail],
    ['auth.tokenResponse', TokenResponseSchema, fixtures.auth.tokenResponse],
    ['auth.tokenResponseOidc', TokenResponseSchema, fixtures.auth.tokenResponseOidc],
    ['device.startResponse', DeviceStartResponseSchema, fixtures.device.startResponse],
    ['device.pollPending', DevicePollResponseSchema, fixtures.device.pollPending],
    ['device.pollAuthorized', DevicePollResponseSchema, fixtures.device.pollAuthorized],
    ['telemetry.installEvent', CliEventInputSchema, fixtures.telemetry.installEvent],
    ['telemetry.auditFailEvent', CliEventInputSchema, fixtures.telemetry.auditFailEvent],
    ['telemetry.authFlowEvent', CliEventInputSchema, fixtures.telemetry.authFlowEvent],
    ['collections.manifest', CollectionManifestSchema, fixtures.collections.manifest],
    ['collections.summary', CollectionSummarySchema, fixtures.collections.summary],
    ['collections.change', ChangeEntrySchema, fixtures.collections.change],
    ['auth.oauthTokenInput', OauthTokenInputSchema, fixtures.auth.oauthTokenInput],
    ['auth.oauthRefreshInput', OauthRefreshInputSchema, fixtures.auth.oauthRefreshInput],
    ['auth.oidcExchangeInput', OidcExchangeInputSchema, fixtures.auth.oidcExchangeInput],
    ['device.startInput', DeviceStartInputSchema, fixtures.device.startInput],
    ['device.pollInput', DevicePollInputSchema, fixtures.device.pollInput],
    ['skills.resolveInput', SkillsResolveInputSchema, fixtures.skills.resolveInput],
    ['skills.resolveResponse', SkillsResolveResponseSchema, fixtures.skills.resolveResponse],
    ['skills.detail', SkillDetailResponseSchema, fixtures.skills.detail],
  ] as const)('%s parses cleanly', (_name, schema, value) => {
    const result = schema.safeParse(value)
    if (!result.success)
      throw new Error(JSON.stringify(result.error.format(), null, 2))
    expect(result.success).toBe(true)
  })
})
