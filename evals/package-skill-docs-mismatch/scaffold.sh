#!/usr/bin/env bash
set -e
mkdir -p src
cat > package.json <<'JSON'
{
  "name": "pricetag",
  "description": "Format a price for display.",
  "type": "module",
  "version": "2.1.0",
  "exports": "./src/index.js",
  "types": "./src/index.d.ts"
}
JSON
# The README documents dollars. The code takes integer cents.
cat > README.md <<'MD'
# pricetag

Format a price for display.

```js
import { formatPrice } from 'pricetag'

formatPrice(12.34) // "$12.34"
formatPrice(5, 'EUR') // "€5.00"
```
MD
printf 'export const formatPrice = (amount, currency = "USD") => new Intl.NumberFormat("en-US", { style: "currency", currency }).format(amount / 100)\n' > src/index.js
printf 'export declare const formatPrice: (amount: number, currency?: string) => string\n' > src/index.d.ts
