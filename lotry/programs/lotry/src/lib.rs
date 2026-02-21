use anchor_lang::prelude::*;
use ephemeral_rollups_sdk::anchor::{commit, delegate, ephemeral};
use ephemeral_rollups_sdk::cpi::{delegate_account, DelegateAccounts, DelegateConfig};
use ephemeral_rollups_sdk::ephem::commit_and_undelegate_accounts;
use ephemeral_vrf_sdk::anchor::vrf;
use ephemeral_vrf_sdk::instructions::{create_request_randomness_ix, RequestRandomnessParams};
use ephemeral_vrf_sdk::types::SerializableAccountMeta;
use std::str::FromStr;

declare_id!("6uuK1kSc5UtnDy7MzhztXQ5fPz3LA6GLwFxxTUvQzC6L");

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
pub const DELEGATION_PROGRAM_ID: &str = "DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh";

// ──────────────────────────────────────────────────────────────────────────────
// Program
// ──────────────────────────────────────────────────────────────────────────────
#[ephemeral_rollups_sdk::anchor::ephemeral]
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

    /// Delegate the lottery pool to the Ephemeral Rollup validator.
    pub fn delegate_lottery(ctx: Context<DelegateLottery>, epoch_id: u64) -> Result<()> {
        let epoch_bytes = epoch_id.to_le_bytes();
        let seeds = &[
            LOTTERY_POOL_SEED,
            &epoch_bytes[..],
        ];
        msg!("Current Program ID: {:?}", crate::id());
        let (derived_pda, derived_bump) = Pubkey::find_program_address(seeds, &crate::id());
        msg!("Manual Derived PDA: {:?} bump: {}", derived_pda, derived_bump);

        msg!("Delegating pool: {:?}", ctx.accounts.lottery_pool.key());
        msg!("Seeds: {:?} {:?}", LOTTERY_POOL_SEED, epoch_bytes);
        
        // Ensure we are delegating to the specific TEE validator we configured
        // in our implementation plan: FnE6VJT5QNZdedZPnCoLsARgBwoE6DeJNjBs2H1gySXA
        let delegate_config = DelegateConfig {
            validator: Some(Pubkey::from_str(TEE_VALIDATOR).map_err(|_| LottryError::InvalidValidator)?),
            ..Default::default()
        };

        // Delegate the account to ER manually
        delegate_account(
            DelegateAccounts {
                payer: &ctx.accounts.authority.to_account_info(),
                pda: &ctx.accounts.lottery_pool.to_account_info(),
                owner_program: &ctx.accounts.owner_program.to_account_info(), 
                buffer: &ctx.accounts.buffer_lottery_pool.to_account_info(),
                delegation_record: &ctx.accounts.delegation_record_lottery_pool.to_account_info(),
                delegation_metadata: &ctx.accounts.delegation_metadata_lottery_pool.to_account_info(),
                delegation_program: &ctx.accounts.delegation_program.to_account_info(), 
                system_program: &ctx.accounts.system_program.to_account_info(),
            },
            seeds,
            delegate_config,
        )?;

        msg!("Lottery pool for epoch {} delegated to ER", epoch_id);
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

    // ── Phase 4: ER Ticket Purchase ──────────────────────────────────────────

    /// Pre-allocates the PlayerTicket on L1 so the ER doesn't have to CPI to SystemProgram
    pub fn init_player_ticket(
        ctx: Context<InitPlayerTicket>,
        _epoch_id: u64,
        _ticket_count: u64,
    ) -> Result<()> {
        msg!("PlayerTicket pre-allocated on L1");
        Ok(())
    }

    /// Delegates the pre-allocated PlayerTicket to the ER
    pub fn delegate_player_ticket(
        ctx: Context<DelegatePlayerTicket>,
        epoch_id: u64,
        ticket_count: u64,
    ) -> Result<()> {
        let pda_signer_seeds: &[&[u8]] = &[
            PLAYER_TICKET_SEED,
            &epoch_id.to_le_bytes(),
            &ticket_count.to_le_bytes(),
        ];

        let delegate_config = ephemeral_rollups_sdk::cpi::DelegateConfig {
            validator: ctx.accounts.validator.as_ref().map(|v| *v.key),
            ..Default::default()
        };

        let delegate_accounts = ephemeral_rollups_sdk::cpi::DelegateAccounts {
            payer: &ctx.accounts.fee_payer.to_account_info(),
            pda: &ctx.accounts.player_ticket.to_account_info(),
            owner_program: &ctx.accounts.owner_program,
            buffer: &ctx.accounts.buffer_player_ticket,
            delegation_record: &ctx.accounts.delegation_record,
            delegation_metadata: &ctx.accounts.delegation_metadata,
            delegation_program: &ctx.accounts.ephemeral_rollups_program,
            system_program: &ctx.accounts.system_program.to_account_info(),
        };

        ephemeral_rollups_sdk::cpi::delegate_account(
            delegate_accounts,
            pda_signer_seeds,
            delegate_config,
        )?;

        msg!("PlayerTicket delegated to ER");
        Ok(())
    }

    /// Executed on the Ephemeral Rollup (ER). Uses pre-allocated L1 PlayerTicket.
    /// Signed only by the ephemeral session key — no SOL transfer (gasless on ER).
    pub fn buy_ticket(
        ctx: Context<BuyTicket>,
        epoch_id: u64,
        ticket_count: u64,
        ticket_data: [u8; 32],
    ) -> Result<()> {
        // Validate session token (standard Anchor accounts, ER remaps ownership)
        let session = &ctx.accounts.session_token;

        // Validate signer and expiry
        require!(
            session.ephemeral_key == ctx.accounts.ephemeral_signer.key(),
            LottryError::InvalidSessionSigner
        );
        require!(
            Clock::get()?.unix_timestamp < session.valid_until,
            LottryError::SessionExpired
        );
        require_keys_eq!(
            session.authority,
            ctx.accounts.authority.key(),
            LottryError::InvalidSessionSigner
        );

        // Access pool and ticket directly (Anchor will remap ownership on ER)
        let pool = &mut ctx.accounts.lottery_pool;

        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.epoch_id == epoch_id, LottryError::EpochMismatch);
        require!(ticket_count == pool.ticket_count, LottryError::EpochMismatch);

        // Update ticket
        let ticket = &mut ctx.accounts.player_ticket;
        ticket.owner = session.authority;
        ticket.epoch_id = epoch_id;
        ticket.ticket_id = ticket_count;
        ticket.ticket_data = ticket_data;

        // Update pool counter
        pool.ticket_count = pool.ticket_count.saturating_add(1);

        msg!(
            "Ticket #{} issued to {} in epoch {}",
            ticket_count,
            session.authority,
            epoch_id
        );
        Ok(())
    }

    // ── Phase 5 ───────────────────────────────────────────────────────────────

    /// Request a VRF winner. Runs on ER. Sends CPI to VRF oracle.
    pub fn request_winner(
        ctx: Context<RequestWinner>,
        epoch_id: u64,
        client_seed: u8,
    ) -> Result<()> {
        let pool_info = &ctx.accounts.lottery_pool;
        let pool: LotteryPool = {
            let data = pool_info.try_borrow_data()?;
            if data.len() < 8 { return Err(anchor_lang::error::ErrorCode::AccountDidNotDeserialize.into()); }
            let mut data_ptr = &data[8..];
            LotteryPool::deserialize(&mut data_ptr)?
        };

        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.ticket_count > 0, LottryError::NoTickets);

        let ix = create_request_randomness_ix(RequestRandomnessParams {
            payer: ctx.accounts.payer.key(),
            oracle_queue: ctx.accounts.oracle_queue.key(),
            callback_program_id: crate::ID,
            callback_discriminator: instruction::ConsumeRandomness::DISCRIMINATOR.to_vec(),
            caller_seed: [client_seed; 32],
            accounts_metas: Some(vec![
                SerializableAccountMeta {
                    pubkey: pool_info.key(),
                    is_signer: false,
                    is_writable: true,
                },
            ]),
            ..Default::default()
        });

        ctx.accounts
            .invoke_signed_vrf(&ctx.accounts.payer.to_account_info(), &ix)?;

        msg!("VRF randomness requested for epoch {}", epoch_id);
        Ok(())
    }

    /// VRF callback — invoked by the VRF oracle program via CPI.
    /// Access-controlled: only the VRF program identity PDA can call this.
    pub fn consume_randomness(
        ctx: Context<ConsumeRandomness>,
        randomness: [u8; 32],
    ) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;

        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.ticket_count > 0, LottryError::NoTickets);
        // Derive winner ticket id in range [0, ticket_count)
        let winner_id = ephemeral_vrf_sdk::rnd::random_u8_with_range(
            &randomness,
            0,
            pool.ticket_count.saturating_sub(1) as u8,
        ) as u64;

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

#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct DelegateLottery<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump,
    )]
    /// CHECK: delegating pda
    pub lottery_pool: Account<'info, LotteryPool>,
    
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Checked by the delegate program — target ER validator
    pub validator: Option<AccountInfo<'info>>,

    /// CHECK: The buffer account - created via CPI
    #[account(mut)]
    pub buffer_lottery_pool: AccountInfo<'info>,

    /// CHECK: The delegation record account - created via CPI
    #[account(mut)]
    pub delegation_record_lottery_pool: AccountInfo<'info>,

    /// CHECK: The delegation metadata account - created via CPI
    #[account(mut)]
    pub delegation_metadata_lottery_pool: AccountInfo<'info>,

    /// CHECK: The delegation program
    #[account(address = ephemeral_rollups_sdk::id())]
    pub delegation_program: AccountInfo<'info>,

    /// CHECK: The owner program
    #[account(address = crate::id())]
    pub owner_program: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
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
#[instruction(epoch_id: u64, ticket_count: u64)]
pub struct InitPlayerTicket<'info> {
    #[account(
        init,
        payer = fee_payer,
        space = PlayerTicket::LEN,
        seeds = [PLAYER_TICKET_SEED, &epoch_id.to_le_bytes(), &ticket_count.to_le_bytes()],
        bump
    )]
    pub player_ticket: Account<'info, PlayerTicket>,
    #[account(mut)]
    pub fee_payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(epoch_id: u64, ticket_count: u64)]
pub struct DelegatePlayerTicket<'info> {
    #[account(
        mut,
        seeds = [PLAYER_TICKET_SEED, &epoch_id.to_le_bytes(), &ticket_count.to_le_bytes()],
        bump
    )]
    pub player_ticket: Account<'info, PlayerTicket>,
    
    #[account(mut)]
    pub fee_payer: Signer<'info>,

    /// CHECK: Checked by the delegate program — target ER validator
    pub validator: Option<AccountInfo<'info>>,

    /// CHECK: The buffer account - created via CPI
    #[account(mut)]
    pub buffer_player_ticket: AccountInfo<'info>,

    /// CHECK: The delegation record account - created via CPI
    #[account(mut)]
    pub delegation_record: AccountInfo<'info>,

    /// CHECK: The delegation metadata account - created via CPI
    #[account(mut)]
    pub delegation_metadata: AccountInfo<'info>,

    /// CHECK: Passed to the CPI
    #[account(address = ephemeral_rollups_sdk::consts::DELEGATION_PROGRAM_ID)]
    pub ephemeral_rollups_program: AccountInfo<'info>,

    /// CHECK: The owner program
    #[account(address = crate::id())]
    pub owner_program: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[ephemeral_rollups_sdk::anchor::action]
#[derive(Accounts)]
#[instruction(epoch_id: u64, ticket_count: u64, ticket_data: [u8; 32])]
pub struct BuyTicket<'info> {
    #[account(mut)]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(mut)]
    pub player_ticket: Account<'info, PlayerTicket>,
    /// CHECK:
    pub authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub session_token: Account<'info, SessionToken>,
    pub ephemeral_signer: Signer<'info>,
    #[account(mut)]
    pub fee_payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// ── Phase 5 ──────────────────────────────────────────────────────────────────

#[vrf]
#[ephemeral_rollups_sdk::anchor::action]
#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct RequestWinner<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    /// CHECK: Manually deserialized to bypass ownership check on ER
    pub lottery_pool: UncheckedAccount<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: oracle queue
    #[account(mut, address = ephemeral_vrf_sdk::consts::DEFAULT_QUEUE)]
    pub oracle_queue: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct ConsumeRandomness<'info> {
    /// The VRF program identity PDA — enforces only the VRF oracle can call this
    #[account(address = ephemeral_vrf_sdk::consts::VRF_PROGRAM_IDENTITY)]
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
    #[msg("Validator pubkey is invalid.")]
    InvalidValidator,
}
