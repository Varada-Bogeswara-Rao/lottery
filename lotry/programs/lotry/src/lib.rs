use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use ephemeral_rollups_sdk::anchor::commit;
use ephemeral_rollups_sdk::cpi::{delegate_account, DelegateAccounts, DelegateConfig};
use ephemeral_rollups_sdk::ephem::commit_and_undelegate_accounts;

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
pub const TEE_VALIDATOR: &str = "MAS1Dt9qreoRMQ14YQuhg8UTZMMzDdKhmkZMECCzk57";
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
    pub fn initialize_lottery(
        ctx: Context<InitializeLottery>,
        epoch_id: u64,
        base_price: u64,
        curve_multiplier: u64,
        tax_rate_bps: u16,
    ) -> Result<()> {
        require!(tax_rate_bps <= 10_000, LottryError::InvalidTaxRate);
        let pool = &mut ctx.accounts.lottery_pool;
        pool.authority = ctx.accounts.authority.key();
        pool.epoch_id = epoch_id;
        pool.ticket_count = 0;
        pool.total_staked_sol = 0;
        pool.tax_treasury_sol = 0;
        pool.base_price = base_price;
        pool.curve_multiplier = curve_multiplier;
        pool.tax_rate_bps = tax_rate_bps;
        pool.is_active = true;
        pool.vrf_request_id = None;
        pool.winner_ticket_id = None;
        msg!(
            "LotteryPool initialized — epoch {} base_price={} curve_multiplier={} tax_rate_bps={}",
            epoch_id,
            base_price,
            curve_multiplier,
            tax_rate_bps
        );
        Ok(())
    }


    // ── Phase 2 ───────────────────────────────────────────────────────────────

    /// Buy ticket credits on L1 with dynamic pricing + tax.
    pub fn buy_ticket_credits(
        ctx: Context<BuyTicketCredits>,
        epoch_id: u64,
        ticket_amount: u64,
    ) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;
        let player_ticket = &mut ctx.accounts.player_ticket;

        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.epoch_id == epoch_id, LottryError::EpochMismatch);
        require!(ticket_amount > 0, LottryError::InvalidTicketAmount);
        require_keys_eq!(
            player_ticket.owner,
            ctx.accounts.buyer.key(),
            LottryError::InvalidTicketOwner
        );

        let current_price = pool.current_price()?;
        let total_price = (current_price as u128)
            .checked_add(
                (pool.curve_multiplier as u128)
                    .checked_mul(ticket_amount as u128)
                    .ok_or(LottryError::MathOverflow)?,
            )
            .ok_or(LottryError::MathOverflow)?;
        require!(total_price <= u64::MAX as u128, LottryError::MathOverflow);
        let total_price_u64 = total_price as u64;

        let tax_amount = total_price
            .checked_mul(pool.tax_rate_bps as u128)
            .ok_or(LottryError::MathOverflow)?
            / 10_000u128;
        require!(tax_amount <= u64::MAX as u128, LottryError::MathOverflow);
        let tax_u64 = tax_amount as u64;
        let net_amount = total_price_u64
            .checked_sub(tax_u64)
            .ok_or(LottryError::MathOverflow)?;

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to: pool.to_account_info(),
            },
        );
        transfer(cpi_ctx, total_price_u64)?;

        pool.total_staked_sol = pool
            .total_staked_sol
            .checked_add(net_amount)
            .ok_or(LottryError::MathOverflow)?;
        pool.tax_treasury_sol = pool
            .tax_treasury_sol
            .checked_add(tax_u64)
            .ok_or(LottryError::MathOverflow)?;
        player_ticket.balance = player_ticket
            .balance
            .checked_add(ticket_amount)
            .ok_or(LottryError::MathOverflow)?;

        msg!(
            "Credits purchased: buyer={} tickets={} total_price={} tax={} net={}",
            ctx.accounts.buyer.key(),
            ticket_amount,
            total_price_u64,
            tax_u64,
            net_amount
        );
        Ok(())
    }

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
        
        // Use the dynamically provided validator account
        let delegate_config = DelegateConfig {
            validator: ctx.accounts.validator.as_ref().map(|v| *v.key),
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
    pub fn init_player_ticket(ctx: Context<InitPlayerTicket>, epoch_id: u64) -> Result<()> {
        let ticket = &mut ctx.accounts.player_ticket;
        ticket.owner = ctx.accounts.authority.key();
        ticket.epoch_id = epoch_id;
        ticket.ticket_id = 0;
        ticket.ticket_data = [0u8; 32];
        ticket.balance = 0;
        ticket.is_active = false;

        msg!("PlayerTicket pre-allocated on L1 for {}", ticket.owner);
        Ok(())
    }

    /// Delegates the pre-allocated PlayerTicket to the ER
    pub fn delegate_player_ticket(
        ctx: Context<DelegatePlayerTicket>,
        epoch_id: u64,
    ) -> Result<()> {
        let auth_key = ctx.accounts.authority.key();
        let epoch_id_bytes = epoch_id.to_le_bytes();
        let pda_signer_seeds: &[&[u8]] = &[
            PLAYER_TICKET_SEED,
            auth_key.as_ref(),
            &epoch_id_bytes,
        ];

        let delegate_config = ephemeral_rollups_sdk::cpi::DelegateConfig {
            validator: ctx.accounts.validator.as_ref().map(|v| *v.key),
            ..Default::default()
        };

        let delegate_accounts = ephemeral_rollups_sdk::cpi::DelegateAccounts {
            payer: &ctx.accounts.authority.to_account_info(),
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
    /// Consumes one pre-paid credit from PlayerTicket.balance.
    pub fn buy_ticket(
        ctx: Context<BuyTicket>,
        epoch_id: u64,
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

        let pool = &mut ctx.accounts.lottery_pool;
        let ticket = &mut ctx.accounts.player_ticket;

        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.epoch_id == epoch_id, LottryError::EpochMismatch);
        require_keys_eq!(ticket.owner, session.authority, LottryError::InvalidTicketOwner);
        require!(ticket.balance > 0, LottryError::InsufficientCredits);
        require!(
            !(ticket.is_active && ticket.epoch_id == epoch_id),
            LottryError::TicketAlreadyActive
        );

        ticket.owner = session.authority;
        ticket.epoch_id = epoch_id;
        ticket.ticket_id = pool.ticket_count;
        ticket.ticket_data = ticket_data;
        ticket.is_active = true;
        ticket.balance = ticket
            .balance
            .checked_sub(1)
            .ok_or(LottryError::MathOverflow)?;

        pool.ticket_count = pool.ticket_count.saturating_add(1);

        msg!(
            "Ticket #{} issued to {} in epoch {}",
            ticket.ticket_id,
            session.authority,
            epoch_id
        );
        Ok(())
    }

    // ── Phase 5 ───────────────────────────────────────────────────────────────

    /// Simple timestamp-based winner selection. Runs on ER with session key auth.
    pub fn request_winner(
        ctx: Context<RequestWinner>,
        epoch_id: u64,
        client_seed: u8,
    ) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;
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

        require!(pool.is_active, LottryError::PoolNotActive);
        require!(pool.ticket_count > 0, LottryError::NoTickets);

        // Pseudo-randomness using timestamp
        let timestamp = Clock::get()?.unix_timestamp as u64;
        let winner_id = timestamp.wrapping_add(client_seed as u64) % pool.ticket_count;

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
    pub fn undelegate_pool<'info>(
        ctx: Context<'_, '_, '_, 'info, UndelegatePool<'info>>,
        _epoch_id: u64,
    ) -> Result<()> {
        require!(!ctx.accounts.lottery_pool.is_active, LottryError::PoolStillActive);

        let pool_info = ctx.accounts.lottery_pool.to_account_info();
        require!(pool_info.is_writable, LottryError::AccountNotWritable);

        let mut accounts_to_commit: Vec<&AccountInfo> = vec![&pool_info];
        for account in ctx.remaining_accounts.iter() {
            require!(account.is_writable, LottryError::AccountNotWritable);
            accounts_to_commit.push(account);
        }

        commit_and_undelegate_accounts(
            &ctx.accounts.payer,
            accounts_to_commit,
            &ctx.accounts.magic_context,
            &ctx.accounts.magic_program,
        )?;

        msg!(
            "LotteryPool epoch {} committed & undelegated",
            ctx.accounts.lottery_pool.epoch_id
        );
        Ok(())
    }

    // ── Phase 7 ───────────────────────────────────────────────────────────────

    /// Claim the prize on L1 after the pool is undelegated.
    pub fn claim_prize(ctx: Context<ClaimPrize>, epoch_id: u64) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;
        let ticket = &mut ctx.accounts.player_ticket;

        require!(!pool.is_active, LottryError::PoolStillActive);
        require!(pool.epoch_id == epoch_id, LottryError::EpochMismatch);
        let winner_id = pool.winner_ticket_id.ok_or(LottryError::WinnerNotSet)?;
        require!(
            ticket.is_active && ticket.epoch_id == epoch_id,
            LottryError::TicketNotActive
        );
        require!(ticket.ticket_id == winner_id, LottryError::NotWinner);
        require_keys_eq!(
            ticket.owner,
            ctx.accounts.winner.key(),
            LottryError::InvalidTicketOwner
        );

        let total_staked = pool.total_staked_sol;
        require!(total_staked > 0, LottryError::NoStakedFunds);

        let tax_amount = (total_staked as u128)
            .checked_mul(pool.tax_rate_bps as u128)
            .ok_or(LottryError::MathOverflow)?
            / 10_000u128;
        require!(tax_amount <= u64::MAX as u128, LottryError::MathOverflow);
        let tax_u64 = tax_amount as u64;
        let payout = total_staked
            .checked_sub(tax_u64)
            .ok_or(LottryError::MathOverflow)?;

        **pool.to_account_info().try_borrow_mut_lamports()? -= payout;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += payout;

        pool.total_staked_sol = 0;
        pool.tax_treasury_sol = pool
            .tax_treasury_sol
            .checked_add(tax_u64)
            .ok_or(LottryError::MathOverflow)?;
        ticket.is_active = false;

        msg!(
            "Prize claimed: winner={} payout={} tax={}",
            ctx.accounts.winner.key(),
            payout,
            tax_u64
        );
        Ok(())
    }

    /// Withdraw accumulated taxes to the treasury wallet (admin-only).
    pub fn withdraw_taxes(ctx: Context<WithdrawTaxes>, epoch_id: u64) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;
        require!(pool.epoch_id == epoch_id, LottryError::EpochMismatch);
        require_keys_eq!(
            pool.authority,
            ctx.accounts.authority.key(),
            LottryError::Unauthorized
        );

        let amount = pool.tax_treasury_sol;
        require!(amount > 0, LottryError::NoTaxes);

        **pool.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.treasury.to_account_info().try_borrow_mut_lamports()? += amount;

        pool.tax_treasury_sol = 0;
        msg!(
            "Taxes withdrawn: treasury={} amount={}",
            ctx.accounts.treasury.key(),
            amount
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
    pub total_staked_sol: u64,     // 8
    pub tax_treasury_sol: u64,     // 8
    pub base_price: u64,           // 8
    pub curve_multiplier: u64,     // 8
    pub tax_rate_bps: u16,         // 2
    pub is_active: bool,           // 1
    pub vrf_request_id: Option<Pubkey>, // 1 + 32
    pub winner_ticket_id: Option<u64>,  // 1 + 8
}

impl LotteryPool {
    pub const LEN: usize = 8 + 32 + 8 + 8 + 8 + 8 + 8 + 8 + 2 + 1 + (1 + 32) + (1 + 8);

    pub fn current_price(&self) -> Result<u64> {
        let price = (self.base_price as u128)
            .checked_add(
                (self.curve_multiplier as u128)
                    .checked_mul(self.total_staked_sol as u128)
                    .ok_or(LottryError::MathOverflow)?,
            )
            .ok_or(LottryError::MathOverflow)?;
        require!(price <= u64::MAX as u128, LottryError::MathOverflow);
        Ok(price as u64)
    }
}

/// Individual participant ticket — shielded in TEE.
#[account]
pub struct PlayerTicket {
    pub owner: Pubkey,         // 32
    pub epoch_id: u64,         // 8
    pub ticket_id: u64,        // 8
    pub ticket_data: [u8; 32], // 32 (hashed/shielded entry)
    pub balance: u64,          // 8
    pub is_active: bool,       // 1
}

impl PlayerTicket {
    pub const LEN: usize = 8 + 32 + 8 + 8 + 32 + 8 + 1;
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
    #[account(address = ephemeral_rollups_sdk::consts::DELEGATION_PROGRAM_ID)]
    pub delegation_program: AccountInfo<'info>,

    /// CHECK: The owner program
    #[account(address = crate::id())]
    pub owner_program: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

// ── Phase 2 ──────────────────────────────────────────────────────────────────

#[derive(Accounts)]
#[instruction(epoch_id: u64, ticket_amount: u64)]
pub struct BuyTicketCredits<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(
        mut,
        seeds = [PLAYER_TICKET_SEED, buyer.key().as_ref(), &epoch_id.to_le_bytes()],
        bump
    )]
    pub player_ticket: Account<'info, PlayerTicket>,
    #[account(mut)]
    pub buyer: Signer<'info>,
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
#[instruction(epoch_id: u64)]
pub struct InitPlayerTicket<'info> {
    #[account(
        init,
        payer = authority,
        space = PlayerTicket::LEN,
        seeds = [PLAYER_TICKET_SEED, authority.key().as_ref(), &epoch_id.to_le_bytes()],
        bump
    )]
    pub player_ticket: Account<'info, PlayerTicket>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct DelegatePlayerTicket<'info> {
    #[account(
        mut,
        seeds = [PLAYER_TICKET_SEED, authority.key().as_ref(), &epoch_id.to_le_bytes()],
        bump
    )]
    pub player_ticket: Account<'info, PlayerTicket>,
    
    #[account(mut)]
    pub authority: Signer<'info>,

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

#[derive(Accounts)]
#[instruction(epoch_id: u64, ticket_data: [u8; 32])]
pub struct BuyTicket<'info> {
    #[account(mut)]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(
        mut,
        seeds = [PLAYER_TICKET_SEED, authority.key().as_ref(), &epoch_id.to_le_bytes()],
        bump
    )]
    pub player_ticket: Account<'info, PlayerTicket>,
    /// CHECK:
    pub authority: UncheckedAccount<'info>,
    pub session_token: Account<'info, SessionToken>,
    pub ephemeral_signer: Signer<'info>,
    pub fee_payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// ── Phase 5 ──────────────────────────────────────────────────────────────────

#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct RequestWinner<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    /// CHECK:
    pub authority: UncheckedAccount<'info>,
    pub session_token: Account<'info, SessionToken>,
    pub ephemeral_signer: Signer<'info>,
}


// ── Phase 6 ──────────────────────────────────────────────────────────────────

// Removed #[commit] macro to allow explicit mutability and configurable Magic IDs 
#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct UndelegatePool<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    
    #[account(mut)] // Payer needs to be mutable for lamport transfers during commit
    pub payer: Signer<'info>,
    
    /// CHECK: Magic context must be mutable for the schedule commit invocation
    #[account(mut)]
    pub magic_context: AccountInfo<'info>,
    
    /// CHECK: Magic program executable
    pub magic_program: AccountInfo<'info>,
}

// ── Phase 7 ──────────────────────────────────────────────────────────────────

#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct ClaimPrize<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(
        mut,
        seeds = [PLAYER_TICKET_SEED, winner.key().as_ref(), &epoch_id.to_le_bytes()],
        bump
    )]
    pub player_ticket: Account<'info, PlayerTicket>,
    #[account(mut)]
    pub winner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(epoch_id: u64)]
pub struct WithdrawTaxes<'info> {
    #[account(
        mut,
        seeds = [LOTTERY_POOL_SEED, &epoch_id.to_le_bytes()],
        bump
    )]
    pub lottery_pool: Account<'info, LotteryPool>,
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(mut)]
    pub treasury: SystemAccount<'info>,
    pub system_program: Program<'info, System>,
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
    #[msg("Invalid tax rate (basis points must be <= 10000).")]
    InvalidTaxRate,
    #[msg("Ticket amount must be greater than zero.")]
    InvalidTicketAmount,
    #[msg("Player ticket does not belong to the buyer.")]
    InvalidTicketOwner,
    #[msg("Insufficient ticket credits.")]
    InsufficientCredits,
    #[msg("Ticket already active for this epoch.")]
    TicketAlreadyActive,
    #[msg("Math overflow.")]
    MathOverflow,
    #[msg("Winner not selected for this epoch.")]
    WinnerNotSet,
    #[msg("Ticket is not active for this epoch.")]
    TicketNotActive,
    #[msg("Caller is not the winner.")]
    NotWinner,
    #[msg("No staked funds available for payout.")]
    NoStakedFunds,
    #[msg("Unauthorized.")]
    Unauthorized,
    #[msg("No taxes available for withdrawal.")]
    NoTaxes,
    #[msg("No tickets in this epoch — cannot request winner.")]
    NoTickets,
    #[msg("Validator pubkey is invalid.")]
    InvalidValidator,
    #[msg("Account must be writable for commit/undelegate.")]
    AccountNotWritable,
}
