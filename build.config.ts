import { defineBuildConfig } from 'obuild/config'

export default defineBuildConfig({
  entries: [
    {
      type: 'bundle',
      input: [
        './src/index.ts',
        './src/cli-entry.ts',
        './src/cli.ts',
        './src/prepare.ts',
        './src/types.ts',
        './src/retriv/worker.ts',
      ],
    },
  ],
})
