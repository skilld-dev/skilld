import { defineBuildConfig } from 'obuild/config'

export default defineBuildConfig({
  entries: [
    {
      type: 'bundle',
      input: [
        './src/wire.ts',
        './src/constants.ts',
        './src/test-fixtures.ts',
      ],
      outDir: './dist',
    },
  ],
})
