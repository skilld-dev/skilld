import { detectCurrentAgent } from 'unagent/env'

export function isRunningInsideAgent(): boolean {
  return !!detectCurrentAgent()
}

export function isInteractive(): boolean {
  if (isRunningInsideAgent())
    return false
  if (process.env.CI)
    return false
  if (!process.stdout.isTTY)
    return false
  return true
}

export function requireInteractive(command: string): void {
  if (!isInteractive()) {
    console.error(`Error: \`skilld ${command}\` requires an interactive terminal`)
    process.exit(1)
  }
}
