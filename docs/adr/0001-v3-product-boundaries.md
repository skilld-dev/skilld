# ADR 0001: v3 product boundaries

Date: 2026-08-20

Status: accepted

## Context

The v2 CLI combines Skill management, source research, generation, Agent execution, indexing, and cache behavior.

Large Agent context windows reduced the value of a bundled generation engine.

The combined runtime also makes management slower and harder to port.

## Decision

The skilld CLI becomes Rust.

Generation moves to visible skilld-maintained Skills and the JavaScript `skilld-harness` package.

skilld-maintained Skills remain usable directly through an Agent.

Direct runs remain user reviewed.

Harness applies deterministic checks and atomic promotion.

`skilld.dev` provides attested Artifact delivery for public and private GitHub Repositories.

Private delivery is transient.

GitHub remains the namespace and source of truth.

The skilld CLI never imports Harness or generation code.

Harness never imports the skilld CLI as a library.

Native and WASIp2 distributions call the same Rust command module.

WASIp2 support ships only after its host interface proof passes.

## Consequences

The v2 JavaScript manager and generation runtime are deleted after replacement parity.

The npm `skilld` package becomes a small artifact loader.

Direct native binaries provide the fastest startup path.

The npm loader retains Node startup cost.

Remote installs fail closed when Artifact delivery is unavailable.

Users may request explicit direct remote access with an unverified source status.

Strict CI rejects unverified remote sources.

`skilld install skilld --global` installs the skilld-maintained Skill for search and install guidance.
