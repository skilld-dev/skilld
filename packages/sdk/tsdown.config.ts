import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: {
    index: 'src/index.ts',
    contract: 'src/contract/index.ts',
  },
  format: 'esm',
  dts: true,
  clean: true,
})
