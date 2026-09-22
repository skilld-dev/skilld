#!/usr/bin/env bash
set -e
mkdir -p src/commands src/core docs dist
cat > package.json <<'JSON'
{
  "name": "invoicer",
  "description": "Render and send invoices from a ledger file.",
  "type": "module",
  "main": "./src/index.ts",
  "scripts": {
    "test": "vitest run",
    "lint": "eslint ."
  }
}
JSON
printf '# invoicer\n\nRender and send invoices from a ledger file.\n' > README.md
printf 'export { renderInvoice } from "./core/render.ts"\nexport { sendInvoice } from "./core/send.ts"\n' > src/index.ts
printf 'export const renderInvoice = (ledger: string): string => ledger.trim()\n' > src/core/render.ts
printf 'export const sendInvoice = async (body: string): Promise<void> => { void body }\n' > src/core/send.ts
printf 'export const runSend = async (): Promise<void> => {}\n' > src/commands/send.ts
printf '# Ledger format\n\nOne invoice per line.\n' > docs/ledger.md
printf 'export const renderInvoice=(l)=>l.trim();\n' > dist/index.js
printf 'export const sendInvoice=async(b)=>{};\n' > dist/send.js
