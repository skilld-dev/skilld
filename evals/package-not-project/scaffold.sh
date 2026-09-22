#!/usr/bin/env bash
set -e
mkdir -p src test
cat > package.json <<'JSON'
{
  "name": "tinyfmt",
  "description": "Format a number as a currency string.",
  "type": "module",
  "version": "1.4.0",
  "exports": "./src/index.js",
  "types": "./src/index.d.ts"
}
JSON
printf '# tinyfmt\n\nFormat a number as a currency string.\n\n## Usage\n\n```js\nimport { formatMoney } from "tinyfmt"\n```\n' > README.md
printf 'export const formatMoney = (cents, currency = "USD") => new Intl.NumberFormat("en-US", { style: "currency", currency }).format(cents / 100)\n' > src/index.js
printf 'export declare const formatMoney: (cents: number, currency?: string) => string\n' > src/index.d.ts
printf 'import { formatMoney } from "../src/index.js"\nconsole.log(formatMoney(1234))\n' > test/format.test.js
