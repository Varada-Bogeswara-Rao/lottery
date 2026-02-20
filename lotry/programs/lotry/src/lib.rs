use anchor_lang::prelude::*;
use ephemeral_rollups_sdk::anchor::{commit, delegate, ephemeral};
use ephemeral_rollups_sdk::cpi::DelegateConfig;
use ephemeral_rollups_sdk::ephem::commit_and_undelegate_accounts;
// VRF SDK imports — enabled in Phase 5
// use ephemeral_vrf_sdk::anchor::vrf;
// use ephemeral_vrf_sdk::instructions::{create_request_randomness_ix, RequestRandomnessParams};
// use ephemeral_vrf_sdk::types::SerializableAccountMeta;

declare_id!("8EfoffNAfiKmbLZYJ6N6YvF7PmRmrfJHoPzGH5jh5jvW");

// ──────────────────────────────────────────────────────────────────────────────
// Seeds
// ──────────────────────────────────────────────────────────────────────────────
pub const LOTTERY_POOL_SEED: &[u8] = b"lottery_pool";
pub const PLAYER_TICKET_SEED: &[u8] = b"player_ticket";
pub const SESSION_SEED: &[u8] = b"session";

// ──────────────────────────────────────────────────────────────────────────────
// TEE / ER validator pubkeys
// ──────────────────────────────────────────────────────────────────────────────
// TEE validator: tee.magicblock.app
pub const TEE_VALIDATOR: &str = "FnE6VJT5QNZdedZPnCoLsARgBwoE6DeJNjBs2H1gySXA";

// ──────────────────────────────────────────────────────────────────────────────
// Program
// ──────────────────────────────────────────────────────────────────────────────
#[ephemeral]
#[program]
pub mod lotry {
    use super::*;

    // ── Phase 1 ───────────────────────────────────────────────────────────────

    /// Create a new lottery epoch on the base layer (L1).
    pub fn initialize_lottery(ctx: Context<InitializeLottery>, epoch_id: u64) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;
        pool.authority = ctx.accounts.authority.key();
        pool.epoch_id = epoch_id;
        pool.ticket_count = 0;
        pool.total_funds = 0;
        pool.is_active = true;
        pool.vrf_request_id = None;
        pool.winner_ticket_id = None;
        msg!("LotteryPool initialized — epoch {}", epoch_id);
        Ok(())
    }

    // ── Phase 2 ───────────────────────────────────────────────────────────────

    /// Delegate LotteryPool to the Ephemeral Rollup (TEE validator).
    pub fn delegate_pool(ctx: Context<DelegatePool>, epoch_id: u64) -> Result<()> {
        let validator: Option<Pubkey> = ctx
            .accounts
            .validator
            .as_ref()
            .map(|v| v.key());

        ctx.accounts.delegate_lottery_pool(
            &ctx.accounts.authority,
            &[
                LOTTERY_POOL_SEED,
                &epoch_id.to_le_bytes(),
            ],
            DelegateConfig {
                validator,
                ..Default::default()
            },
        )?;
        msg!("LotteryPool epoch {} delegated to ER", epoch_id);
        Ok(())
    }

    // ── Phase 3 ───────────────────────────────────────────────────────────────

    /// Issue a session key for frictionless high-frequency ticket purchases.
    /// Must be signed by the user's primary wallet.
    pub fn issue_session(
        ctx: Context<IssueSession>,
        ephemeral_key: Pubkey,
        valid_until: i64,
    ) -> Result<()> {
        require!(
            valid_until > Clock::get()?.unix_timestamp,
            LottryError::InvalidExpiry
        );
        let session = &mut ctx.accounts.session_token;
        session.authority = ctx.accounts.authority.key();
        session.ephemeral_key = ephemeral_key;
        session.valid_until = valid_until;
        msg!(
            "SessionToken issued: ephemeral_key={} valid_until={}",
            ephemeral_key,
            valid_until
        );
        Ok(())
    }

    // ── Phase 4 ───────────────────────────────────────────────────────────────

    /// Buy a ticket. Runs in the Ephemeral Rollup / TEE.
    /// Signed only by the ephemeral session key — no SOL transfer (gasless on ER).
    pub fn buy_ticket(
        ctx: Context<BuyTicket>,
        epoch_id: u64,
        ticket_data: [u8; 32],
    ) -> Result<()> {
        // Validate session token
        let session = &ctx.accounts.session_token;
        require!(
            session.ephemeral_key == ctx.accounts.ephemeral_signer.key(),
            LottryError::InvalidSessionSigner
        );
        require!(
            Clock::get()?.unix_timestamp < session.valid_until,
            LottryError::SessionExpired
        );

        // Validate pool
        let pool = &mut ctx.accounts.lottery_pool;
        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.epoch_id == epoch_id, LottryError::EpochMismatch);

        // Record ticket
        let ticket = &mut ctx.accounts.player_ticket;
        ticket.owner = session.authority;
        ticket.epoch_id = epoch_id;
        ticket.ticket_data = ticket_data;
        ticket.ticket_id = pool.ticket_count;

        pool.ticket_count += 1;

        msg!(
            "Ticket #{} issued to {} in epoch {}",
            ticket.ticket_id,
            ticket.owner,
            epoch_id
        );
        Ok(())
    }

    // ── Phase 5 ───────────────────────────────────────────────────────────────

    /// Request a VRF winner. Runs on ER. Sends CPI to VRF oracle.
    /// TODO Phase 5: re-enable ephemeral-vrf-sdk and implement VRF CPI.
    pub fn request_winner(
        ctx: Context<RequestWinner>,
        epoch_id: u64,
        _client_seed: u8,
    ) -> Result<()> {
        let pool = &ctx.accounts.lottery_pool;
        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.ticket_count > 0, LottryError::NoTickets);
        msg!("VRF randomness requested for epoch {} (stub)", epoch_id);
        Ok(())
    }

    /// VRF callback — invoked by the VRF oracle program via CPI.
    /// TODO Phase 5: validate VRF_PROGRAM_IDENTITY signer and derive winner.
    pub fn consume_randomness(
        ctx: Context<ConsumeRandomness>,
        randomness: [u8; 32],
    ) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;
        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.ticket_count > 0, LottryError::NoTickets);

        // Simple modulo winner selection (Phase 5 will use ephemeral_vrf_sdk::rnd)
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&randomness[..8]);
        let rnd = u64::from_le_bytes(bytes);
        let winner_id = rnd % pool.ticket_count;

        pool.winner_ticket_id = Some(winner_id);
        pool.is_active = false;

        msg!(
            "Winner ticket #{} selected for epoch {}",
            winner_id,
            pool.epoch_id
        );
        Ok(())
    }

    // ── Phase 6 ───────────────────────────────────────────────────────────────

    /// Commit final state to L1 and undelegate the LotteryPool from the ER.
    pub fn undelegate_pool(ctx: Context<UndelegatePool>, _epoch_id: u64) -> Result<()> {
        require!(!ctx.accounts.lottery_pool.is_active, LottryError::PoolStillActive);

        commit_and_undelegate_accounts(
            &ctx.accounts.payer,
            vec![&ctx.accounts.lottery_pool.to_account_info()],
            &ctx.accounts.magic_context,
            &ctx.accounts.magic_program,
        )?;

        msg!(
            "LotteryPool epoch {} committed & undelegated",
            ctx.accounts.lottery_pool.epoch_id
        );
        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Account Structs
// ──────────────────────────────────────────────────────────────────────────────

/// Global lottery pool — tracks epoch state.
#[account]
pub struct LotteryPool {
    pub authority: Pubkey,         // 32
    pub epoch_id: u64,             // 8
    pub ticket_count: u64,         // 8
    pub total_funds: u64,          // 8
    pub is_active: bool,           // 1
    pub vrf_request_id: Option<Pubkey>, // 1 + 32
    pub winner_ticket_id: Option<u64>,  // 1 + 8
}

impl LotteryPool {
    pub const LEN: usize = 8 + 32 + 8 + 8 + 8 + 1 + (1 + 32) + (1 + 8);
}

/// Individual participant ticket — shielded in TEE.
#[account]
pub struct PlayerTicket {
    pub owner: Pubkey,        // 32
    pub epoch_id: u64,        // 8
    pub ticket_id: u64,       // 8
    pub ticket_data: [u8; 32], // 32 (hashed/shielded entry)
}

impl PlayerTicket {
    pub const LEN: usize = 8 + 32 + 8 + 8 + 32;
}

/// Session token — secondary signer PDA for frictionless UX.
#[account]
pub struct SessionToken {
    pub authority: Pubkey,     // 32 — primary wallet
    pub ephemeral_key: Pubkey, // 32 — temp client-side keypair
    pub valid_until: i64,      // 8  — unix timestamp expiry
}

impl SessionToken {
    pub const LEN: usize = 8 + 32 + 32 + 8;
}

// ──────────────────────────────────────────────────────────────────────────────
// Contexts
// ──────────────────────────────────────────────────────────────────────────────

// ── Phase 1 ──────────────────────────────────────────────────────────────────

#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct InitializeLottery<'info> {
    #[account(
        init,
        payer = authority,
        space = LotteryPool::LEN,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// ── Phase 2 ──────────────────────────────────────────────────────────────────

#[delegate]
#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct DelegatePool<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump,
        del
    )]
    pub lottery_pool: AccountInfo<'info>,
    #[account(mut)]
    pub authority: Signer<'info>,
    /// CHECK: Checked by the delegate program — target ER validator
    pub validator: Option<AccountInfo<'info>>,
}

// ── Phase 3 ──────────────────────────────────────────────────────────────────

#[derive(Accounts)]
#[instruction(ephemeral_key: Pubkey, valid_until: i64)]
pub struct IssueSession<'info> {
    #[account(
        init,
        payer = authority,
        space = SessionToken::LEN,
        seeds = [SESSION_SEED, authority.key().as_ref(), ephemeral_key.as_ref()],
        bump
    )]
    pub session_token: Account<'info, SessionToken>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// ── Phase 4 ──────────────────────────────────────────────────────────────────

#[derive(Accounts)]
#[instruction(epoch_id: u64, ticket_data: [u8; 32])]
pub struct BuyTicket<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(
        init,
        payer = fee_payer,
        space = PlayerTicket::LEN,
        seeds = [
            PLAYER_TICKET_SEED,
            &epoch_id.to_le_bytes(),
            &lottery_pool.ticket_count.to_le_bytes()
        ],
        bump
    )]
    pub player_ticket: Account<'info, PlayerTicket>,
    /// SessionToken PDA — ephemeral_key validated against ephemeral_signer
    #[account(
        seeds = [SESSION_SEED, session_token.authority.as_ref(), ephemeral_signer.key().as_ref()],
        bump
    )]
    pub session_token: Account<'info, SessionToken>,
    /// The session keypair — must be a signer of this transaction
    pub ephemeral_signer: Signer<'info>,
    /// Fee payer on ER (can be a relayer for gasless UX)
    #[account(mut)]
    pub fee_payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// ── Phase 5 ──────────────────────────────────────────────────────────────────

// TODO Phase 5: add #[vrf] macro once ephemeral-vrf-sdk build issue is resolved
#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct RequestWinner<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: Placeholder oracle queue — will bind to DEFAULT_QUEUE in Phase 5
    #[account(mut)]
    pub oracle_queue: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct ConsumeRandomness<'info> {
    /// CHECK: Will be constrained to VRF_PROGRAM_IDENTITY in Phase 5
    pub vrf_program_identity: Signer<'info>,
    #[account(mut)]
    pub lottery_pool: Account<'info, LotteryPool>,
}

// ── Phase 6 ──────────────────────────────────────────────────────────────────

#[commit]
#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct UndelegatePool<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(mut)]
    pub payer: Signer<'info>,
    // magic_context and magic_program are injected by the #[commit] macro
}

// ──────────────────────────────────────────────────────────────────────────────
// Errors
// ──────────────────────────────────────────────────────────────────────────────

#[error_code]
pub enum LottryError {
    #[msg("Session token has expired.")]
    SessionExpired,
    #[msg("Signer does not match the session ephemeral key.")]
    InvalidSessionSigner,
    #[msg("Lottery pool is not active.")]
    PoolNotActive,
    #[msg("Lottery pool still active — settle randomness first.")]
    PoolStillActive,
    #[msg("Epoch ID mismatch.")]
    EpochMismatch,
    #[msg("Expiry timestamp must be in the future.")]
    InvalidExpiry,
    #[msg("No tickets in this epoch — cannot request winner.")]
    NoTickets,
}
