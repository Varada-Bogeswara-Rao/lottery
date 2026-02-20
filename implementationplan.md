Shielded Micro-Lotteries — Implementation Plan
This plan covers the on-chain Rust programs and TypeScript integration tests for the Shielded Micro-Lotteries platform. It reflects verified, live MagicBlock SDK APIs sourced directly from the official documentation before writing any code.

⚠️ Key Corrections vs. 
info.md
These are places where 
info.md
's assumptions diverge from the actual current SDK:

WARNING

Bolt ECS is NOT used. After reviewing the live docs, the current MagicBlock approach uses plain Anchor programs annotated with #[ephemeral], #[delegate], #[commit], and #[vrf] macros from ephemeral-rollups-sdk and ephemeral_vrf_sdk. There is no #[component] or #[system] Bolt macro in the current PER/ER examples. The Bolt CLI is still used for scaffolding/testing (bolt init, bolt test) but the smart contracts use Anchor directly.

WARNING

VRF callback takes [u8; 32] not [u8; 64]. The SDK callback signature is fn callback_roll_dice(ctx, randomness: [u8; 32]). The helper ephemeral_vrf_sdk::rnd::random_u8_with_range(&randomness, min, max) is used to derive the winner — not manual u64 modulo math on 8 bytes.

WARNING

PER uses a Permission Program, not bare TEE routing. Private Ephemeral Rollups require CPIs to a PERMISSION_PROGRAM_ID via CreatePermissionCpiBuilder and UpdatePermissionCpiBuilder to control who can read state. This is a separate SDK concern not described in 
info.md
.

IMPORTANT

TEE Validator Pubkey: FnE6VJT5QNZdedZPnCoLsARgBwoE6DeJNjBs2H1gySXA (endpoint: tee.magicblock.app). This is the specific validator to target in the delegation config for shielded execution.

IMPORTANT

Delegation Program ID: DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh (unchanged, matches 
info.md
).

Workspace Setup
Project Structure
Lotry/
├── programs/
│   └── lotry/           # Single Anchor program with all instructions
│       └── src/lib.rs
├── tests/
│   └── lotry.ts         # Mocha/Chai integration tests (phase by phase)
├── Anchor.toml
├── Cargo.toml
└── package.json
Rationale: Given the current SDK model uses a single Anchor program annotated with #[ephemeral], splitting into multiple Bolt component/system programs is not aligned with the actual SDK. We use a single program workspace for clarity and maintainability.

Dependencies (Cargo.toml)
toml
[dependencies]
anchor-lang = "0.32.1"
ephemeral-rollups-sdk = { version = "0.6.5", features = ["anchor"] }
ephemeral_vrf_sdk = { version = "*", features = ["anchor"] }
TypeScript Dependencies (package.json)
json
{
  "@coral-xyz/anchor": "^0.32.1",
  "@magicblock-labs/bolt-sdk": "latest",
  "mocha": "^10",
  "chai": "^5",
  "ts-mocha": "^10"
}
Proposed Changes
Phase 1 — Core State + InitializeLottery
[NEW] 
lib.rs
Defines the single Anchor program with #[ephemeral] top-level macro.

Account Structures:

LotteryPool: authority: Pubkey, epoch_id: u64, ticket_count: u64, total_funds: u64, is_active: bool, vrf_request_id: Option<Pubkey>
PlayerTicket: owner: Pubkey, epoch_id: u64, ticket_data: [u8; 32] (hashed entry)
SessionToken: authority: Pubkey, ephemeral_key: Pubkey, valid_until: i64
Instructions (Phase 1):

initialize_lottery(epoch_id) — creates LotteryPool PDA on L1. Seeds: [b"lottery_pool", epoch_id.to_le_bytes()]
[NEW] 
tests/lotry.ts
Phase 1 test asserts LotteryPool is initialized with correct fields on local validator.

Phase 2 — Delegation to ER
New instruction:

delegate_pool(epoch_id) — uses #[delegate] macro on context, calls ctx.accounts.delegate_pda(payer, seeds, DelegateConfig { validator: Some(TEE_VALIDATOR_PUBKEY), ..Default::default() })
Imports:

rust
use ephemeral_rollups_sdk::anchor::{commit, delegate, ephemeral};
use ephemeral_rollups_sdk::cpi::DelegateConfig;
use ephemeral_rollups_sdk::ephem::{commit_accounts, commit_and_undelegate_accounts};
Test (Phase 2): Calls delegate_pool, then verifies the account is owned by the delegation program.

Phase 3 — Session Key Issuance
New instruction:

issue_session(ephemeral_key: Pubkey, valid_until: i64) — initializes SessionToken PDA on L1. Seeds: [b"session", authority.key().as_ref(), ephemeral_key.as_ref()]
Signed by the primary authority wallet.
Test (Phase 3): Generates a new Keypair in-test, derives PDA, calls issue_session, asserts valid_until stored correctly.

Phase 4 — BuyTicket (TEE Execution)
New instruction:

buy_ticket(epoch_id, ticket_data: [u8; 32]) — runs on ER/TEE.
Validates SessionToken (checks ephemeral_key matches signer, checks valid_until > Clock::get().unix_timestamp).
Increments LotteryPool.ticket_count.
Creates new PlayerTicket PDA.
No SOL transfer (gasless on ER).
Custom errors:

rust
#[error_code]
pub enum LottryError {
    SessionExpired,
    InvalidSessionSigner,
    PoolNotActive,
}
Test (Phase 4): Signs only with the ephemeral keypair, routes to ER RPC (tee.magicblock.app), asserts ticket_count incremented.

Phase 5 — VRF: Request + Consume Randomness
New instructions:

request_winner(epoch_id, client_seed: u8) — uses #[vrf] macro on context. Calls create_request_randomness_ix(RequestRandomnessParams { callback_discriminator: instruction::ConsumeRandomness::DISCRIMINATOR.to_vec(), accounts_metas: [...LotteryPool...], ... }) then ctx.accounts.invoke_signed_vrf(...).
consume_randomness(randomness: [u8; 32]) — callback invoked by VRF oracle. Derives winner via ephemeral_vrf_sdk::rnd::random_u64_with_range(&randomness, 0, pool.ticket_count - 1). Sets is_active = false.
Access control: #[account(address = ephemeral_vrf_sdk::consts::VRF_PROGRAM_IDENTITY)] pub vrf_program_identity: Signer<'info>.
Imports:

rust
use ephemeral_vrf_sdk::anchor::vrf;
use ephemeral_vrf_sdk::instructions::{create_request_randomness_ix, RequestRandomnessParams};
use ephemeral_vrf_sdk::types::SerializableAccountMeta;
Test (Phase 5): Calls request_winner on devnet/localnet ER, waits ~3s, re-fetches LotteryPool and asserts is_active == false and a winner ticket ID is recorded.

Phase 6 — Settle + Undelegate
New instruction:

undelegate_pool(epoch_id) — uses #[commit] on context. Calls commit_and_undelegate_accounts(payer, vec![&pool.to_account_info()], magic_context, magic_program). Also sets game.exit(&crate::ID)? before commit (as in PER example).
Context:

rust
#[commit]
#[derive(Accounts)]
pub struct UndelegatePool<'info> {
    #[account(mut, seeds = [...], bump)]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(mut)] pub payer: Signer<'info>,
    // magic_context and magic_program injected by #[commit] macro
}
Test (Phase 6): Full end-to-end test. Orchestrates phases 1–6 sequentially. Times the ER phase vs L1 phase. After undelegate_pool, fetches LotteryPool from L1 RPC and asserts it is settled, is_active == false.

Verification Plan
Automated Tests (run after each phase)
bash
# Spin up local validator + local ER
bolt test
# OR:
anchor test --skip-build --skip-deploy --skip-local-validator
NOTE

bolt test is required for phases that need a local ER (phases 2+). It starts both solana-test-validator and @magicblock-labs/ephemeral-validator concurrently.

Phase-by-phase test checklist:

Phase 1: LotteryPool account exists with correct epoch_id, is_active = true, ticket_count = 0
Phase 2: After delegation, account owner = DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh
Phase 3: SessionToken PDA has correct ephemeral_key and valid_until
Phase 4: ticket_count incremented; transaction signed only by ephemeral key
Phase 5: is_active == false after ~3s wait; winner ID within [0, ticket_count)
Phase 6: L1 fetch of LotteryPool returns settled state; ER fetch returns 404/unowned
Manual Verification
After Phase 2: Check Solana Explorer for the LotteryPool account — its owner should show the Delegation Program, not your program ID.
After Phase 6: Check Solana Explorer at the L1 RPC endpoint — the LotteryPool account should be owned by your program again, with final settled state.

Comment
Ctrl+Alt+M

