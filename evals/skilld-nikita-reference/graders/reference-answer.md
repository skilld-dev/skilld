---
type: regex
target: last_message
pattern: '^(?=[\s\S]*community.{0,30}codes?)(?=[\s\S]*QR)(?=[\s\S]*deep.{0,10}links?)(?=[\s\S]*phone.{0,20}number)'
flags: i
match: contains
weight: 2
---
