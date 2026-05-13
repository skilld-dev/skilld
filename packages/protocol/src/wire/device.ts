/**
 * RFC 8628 device-flow wire shapes.
 */

import { z } from 'zod'
import { TokenResponseSchema } from './auth.ts'

/**
 * Inputs to `POST /api/cli/device/start` and `POST /api/cli/device/poll`.
 */
export const DeviceStartInputSchema = z.object({
  cli_version: z.string().max(32),
  machine_hint: z.string().max(128).optional(),
})

export const DevicePollInputSchema = z.object({
  device_code: z.string().min(16),
})

export const DeviceStartResponseSchema = z.object({
  device_code: z.string(),
  user_code: z.string(),
  verification_uri: z.string().url(),
  interval: z.number().int().positive(),
  expires_in: z.number().int().positive(),
})

export const DevicePollResponseSchema = z.object({
  status: z.enum(['pending', 'authorized', 'expired', 'denied']),
  tokens: TokenResponseSchema.optional(),
})

export type DeviceStartInput = z.infer<typeof DeviceStartInputSchema>
export type DevicePollInput = z.infer<typeof DevicePollInputSchema>
export type DeviceStartResponse = z.infer<typeof DeviceStartResponseSchema>
export type DevicePollResponse = z.infer<typeof DevicePollResponseSchema>
