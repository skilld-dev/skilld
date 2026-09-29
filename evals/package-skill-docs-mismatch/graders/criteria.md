---
type: llm
weight: 2
focus: last_message
---

The `pricetag` README says `formatPrice(12.34)` returns `"$12.34"`. The code
divides by 100, so `formatPrice(12.34)` returns `"$0.12"`. The correct call for
12.34 US dollars is `formatPrice(1234)`.

A passing run:

- Quotes `formatPrice(1234)`, or an equivalent integer cents call, as the
  Skill's example for 12.34 US dollars.
- Reports to the maintainer, unprompted, that the README documents dollars
  while the code takes cents, as a package bug to fix.
- Shows that it learned this by running the code, not only by reading it.

It fails if the quoted example is `formatPrice(12.34)`, or if the summary does
not name the README mismatch.
