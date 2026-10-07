import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['tests/loader/**/*.test.{mjs,ts}', 'tests/evals/**/*.test.ts'],
    reporters: 'dot',
  },
})
