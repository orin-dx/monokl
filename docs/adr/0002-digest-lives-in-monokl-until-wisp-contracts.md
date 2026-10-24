# 2. Define Digest in monokl until wisp-contracts exists

**Status:** Accepted **Date:** 2026-10-23

## Context

`08-library-session-api.md` §8.4 replaces bare-hex `ContentHash` with an algorithm-prefixed `Digest` and places it in `wisp-contracts`, an MIT crate in the Wisp repository. That crate does not exist: it is not on crates.io and the Wisp repository has no such crate. FileAnalysis, the cache and disk persistence all key on the digest, and §14 says nothing should be built on bare hex.

## Decision

`Digest` and `DigestAlgorithm` live in `monokl::digest` with exactly the §8.4 shape: public `algorithm` and `bytes` fields, an exhaustive `DigestAlgorithm { Sha256, Blake3 }` (no `#[non_exhaustive]`), `algorithm:hex` text, lowercase-only hex. When `wisp-contracts` ships `Digest`, monokl re-exports it from `monokl::digest` and deletes its own.

## Consequences

- Cache, FileAnalysis and persistence work can start now on the type they will keep.
- The move is a re-export, not a format change: the text and serde forms are fixed by §8.4, so cache entries written before the move still parse after it.
- Until then the type is FSL-1.1-MIT inside monokl, so a third-party harness plugin cannot link it. Nothing consumes it outside monokl yet.

## Alternatives considered

- **Wait for wisp-contracts.** Blocks tracks 14, 16 and 17 on a crate nobody has started.
- **Keep bare-hex `ContentHash` for now.** §14's warning: every cache entry written would need migrating when `Digest` lands.
