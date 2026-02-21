# Change Log

## 2026-02-21
- Added Phase 4 integration test to `tests/lotry.ts`, covering gasless ticket purchase on ER using session key signer and manual account decoding post-delegation.
- Introduced Devnet-specific provider/program handles for L1-only calls and maintained ER provider for rollup interactions.
- Bumped working `epochId` to 9 to avoid PDA reuse conflicts on Devnet.
