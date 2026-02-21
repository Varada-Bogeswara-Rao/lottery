pub mod lotry {
    use super::*;
    /// Create a new lottery epoch on the base layer (L1).
    pub fn initialize_lottery(
        ctx: Context<InitializeLottery>,
        epoch_id: u64,
    ) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;
        pool.authority = ctx.accounts.authority.key();
        pool.epoch_id = epoch_id;
        pool.ticket_count = 0;
        pool.total_funds = 0;
        pool.is_active = true;
        pool.vrf_request_id = None;
        pool.winner_ticket_id = None;
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!("LotteryPool initialized — epoch {0}", epoch_id),
                )
            }),
        );
        Ok(())
    }
    /// Delegate the lottery pool to the Ephemeral Rollup validator.
    pub fn delegate_lottery(ctx: Context<DelegateLottery>, epoch_id: u64) -> Result<()> {
        let epoch_bytes = epoch_id.to_le_bytes();
        let seeds = &[LOTTERY_POOL_SEED, &epoch_bytes[..]];
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!("Current Program ID: {0:?}", crate::id()),
                )
            }),
        );
        let (derived_pda, derived_bump) = Pubkey::find_program_address(
            seeds,
            &crate::id(),
        );
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!(
                        "Manual Derived PDA: {0:?} bump: {1}",
                        derived_pda,
                        derived_bump,
                    ),
                )
            }),
        );
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!(
                        "Delegating pool: {0:?}",
                        ctx.accounts.lottery_pool.key(),
                    ),
                )
            }),
        );
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!("Seeds: {0:?} {1:?}", LOTTERY_POOL_SEED, epoch_bytes),
                )
            }),
        );
        let delegate_config = DelegateConfig {
            validator: Some(
                Pubkey::from_str(TEE_VALIDATOR)
                    .map_err(|_| LottryError::InvalidValidator)?,
            ),
            ..Default::default()
        };
        delegate_account(
            DelegateAccounts {
                payer: &ctx.accounts.authority.to_account_info(),
                pda: &ctx.accounts.lottery_pool.to_account_info(),
                owner_program: &ctx.accounts.owner_program.to_account_info(),
                buffer: &ctx.accounts.buffer_lottery_pool.to_account_info(),
                delegation_record: &ctx
                    .accounts
                    .delegation_record_lottery_pool
                    .to_account_info(),
                delegation_metadata: &ctx
                    .accounts
                    .delegation_metadata_lottery_pool
                    .to_account_info(),
                delegation_program: &ctx.accounts.delegation_program.to_account_info(),
                system_program: &ctx.accounts.system_program.to_account_info(),
            },
            seeds,
            delegate_config,
        )?;
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!("Lottery pool for epoch {0} delegated to ER", epoch_id),
                )
            }),
        );
        Ok(())
    }
    /// Issue a session key for frictionless high-frequency ticket purchases.
    /// Must be signed by the user's primary wallet.
    pub fn issue_session(
        ctx: Context<IssueSession>,
        ephemeral_key: Pubkey,
        valid_until: i64,
    ) -> Result<()> {
        if !(valid_until > Clock::get()?.unix_timestamp) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::InvalidExpiry.name(),
                    error_code_number: LottryError::InvalidExpiry.into(),
                    error_msg: LottryError::InvalidExpiry.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 103u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        let session = &mut ctx.accounts.session_token;
        session.authority = ctx.accounts.authority.key();
        session.ephemeral_key = ephemeral_key;
        session.valid_until = valid_until;
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!(
                        "SessionToken issued: ephemeral_key={0} valid_until={1}",
                        ephemeral_key,
                        valid_until,
                    ),
                )
            }),
        );
        Ok(())
    }
    /// Buy a ticket. Runs in the Ephemeral Rollup / TEE.
    /// Signed only by the ephemeral session key — no SOL transfer (gasless on ER).
    pub fn buy_ticket(
        ctx: Context<BuyTicket>,
        epoch_id: u64,
        ticket_count: u64,
        ticket_data: [u8; 32],
    ) -> Result<()> {
        let session_info = ctx.accounts.session_token.to_account_info();
        let session: SessionToken = {
            let data = session_info.try_borrow_data()?;
            let mut data_ptr: &[u8] = &data;
            SessionToken::try_deserialize(&mut data_ptr)?
        };
        if !(session.ephemeral_key == ctx.accounts.ephemeral_signer.key()) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::InvalidSessionSigner.name(),
                    error_code_number: LottryError::InvalidSessionSigner.into(),
                    error_msg: LottryError::InvalidSessionSigner.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 138u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        if !(Clock::get()?.unix_timestamp < session.valid_until) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::SessionExpired.name(),
                    error_code_number: LottryError::SessionExpired.into(),
                    error_msg: LottryError::SessionExpired.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 142u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        if session.authority != ctx.accounts.authority.key() {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                        error_name: LottryError::InvalidSessionSigner.name(),
                        error_code_number: LottryError::InvalidSessionSigner.into(),
                        error_msg: LottryError::InvalidSessionSigner.to_string(),
                        error_origin: Some(
                            anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                                filename: "programs/lotry/src/lib.rs",
                                line: 146u32,
                            }),
                        ),
                        compared_values: None,
                    })
                    .with_pubkeys((session.authority, ctx.accounts.authority.key())),
            );
        }
        let pool_info = ctx.accounts.lottery_pool.to_account_info();
        let mut pool: LotteryPool = {
            let data = pool_info.try_borrow_data()?;
            let mut data_ptr: &[u8] = &data;
            LotteryPool::try_deserialize(&mut data_ptr)?
        };
        if !(pool.is_active) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::PoolNotActive.name(),
                    error_code_number: LottryError::PoolNotActive.into(),
                    error_msg: LottryError::PoolNotActive.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 160u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        if !(pool.epoch_id == epoch_id) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::EpochMismatch.name(),
                    error_code_number: LottryError::EpochMismatch.into(),
                    error_msg: LottryError::EpochMismatch.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 161u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        if !(ticket_count == pool.ticket_count) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::EpochMismatch.name(),
                    error_code_number: LottryError::EpochMismatch.into(),
                    error_msg: LottryError::EpochMismatch.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 162u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        let ticket_info = ctx.accounts.player_ticket.to_account_info();
        let ticket_seeds = &[
            PLAYER_TICKET_SEED,
            &epoch_id.to_le_bytes(),
            &ticket_count.to_le_bytes(),
        ];
        let (expected_ticket, bump) = Pubkey::find_program_address(
            ticket_seeds,
            ctx.program_id,
        );
        if expected_ticket != ticket_info.key() {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                        error_name: LottryError::EpochMismatch.name(),
                        error_code_number: LottryError::EpochMismatch.into(),
                        error_msg: LottryError::EpochMismatch.to_string(),
                        error_origin: Some(
                            anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                                filename: "programs/lotry/src/lib.rs",
                                line: 172u32,
                            }),
                        ),
                        compared_values: None,
                    })
                    .with_pubkeys((expected_ticket, ticket_info.key())),
            );
        }
        {
            let mut data = ticket_info.try_borrow_mut_data()?;
            let mut data_ptr: &mut [u8] = &mut data;
            let ticket = PlayerTicket {
                owner: session.authority,
                epoch_id,
                ticket_id: ticket_count,
                ticket_data,
            };
            ticket.try_serialize(&mut data_ptr)?;
        }
        pool.ticket_count = pool.ticket_count.saturating_add(1);
        {
            let mut data = pool_info.try_borrow_mut_data()?;
            let mut data_ptr: &mut [u8] = &mut data;
            pool.try_serialize(&mut data_ptr)?;
        }
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!(
                        "Ticket #{0} issued to {1} in epoch {2}",
                        ticket_count,
                        session.authority,
                        epoch_id,
                    ),
                )
            }),
        );
        Ok(())
    }
    /// Request a VRF winner. Runs on ER. Sends CPI to VRF oracle.
    pub fn request_winner(
        ctx: Context<RequestWinner>,
        epoch_id: u64,
        client_seed: u8,
    ) -> Result<()> {
        let pool_info = &ctx.accounts.lottery_pool;
        let pool: LotteryPool = {
            let data = pool_info.try_borrow_data()?;
            if data.len() < 8 {
                return Err(
                    anchor_lang::error::ErrorCode::AccountDidNotDeserialize.into(),
                );
            }
            let mut data_ptr = &data[8..];
            LotteryPool::deserialize(&mut data_ptr)?
        };
        if !(pool.is_active) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::PoolNotActive.name(),
                    error_code_number: LottryError::PoolNotActive.into(),
                    error_msg: LottryError::PoolNotActive.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 223u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        if !(pool.ticket_count > 0) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::NoTickets.name(),
                    error_code_number: LottryError::NoTickets.into(),
                    error_msg: LottryError::NoTickets.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 224u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        let ix = create_request_randomness_ix(RequestRandomnessParams {
            payer: ctx.accounts.payer.key(),
            oracle_queue: ctx.accounts.oracle_queue.key(),
            callback_program_id: crate::ID,
            callback_discriminator: instruction::ConsumeRandomness::DISCRIMINATOR
                .to_vec(),
            caller_seed: [client_seed; 32],
            accounts_metas: Some(
                <[_]>::into_vec(
                    ::alloc::boxed::box_new([
                        SerializableAccountMeta {
                            pubkey: pool_info.key(),
                            is_signer: false,
                            is_writable: true,
                        },
                    ]),
                ),
            ),
            ..Default::default()
        });
        ctx.accounts.invoke_signed_vrf(&ctx.accounts.payer.to_account_info(), &ix)?;
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!("VRF randomness requested for epoch {0}", epoch_id),
                )
            }),
        );
        Ok(())
    }
    /// VRF callback — invoked by the VRF oracle program via CPI.
    /// Access-controlled: only the VRF program identity PDA can call this.
    pub fn consume_randomness(
        ctx: Context<ConsumeRandomness>,
        randomness: [u8; 32],
    ) -> Result<()> {
        let pool = &mut ctx.accounts.lottery_pool;
        if !(pool.is_active) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::PoolNotActive.name(),
                    error_code_number: LottryError::PoolNotActive.into(),
                    error_msg: LottryError::PoolNotActive.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 257u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        if !(pool.ticket_count > 0) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::NoTickets.name(),
                    error_code_number: LottryError::NoTickets.into(),
                    error_msg: LottryError::NoTickets.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 258u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        let winner_id = ephemeral_vrf_sdk::rnd::random_u8_with_range(
            &randomness,
            0,
            pool.ticket_count.saturating_sub(1) as u8,
        ) as u64;
        pool.winner_ticket_id = Some(winner_id);
        pool.is_active = false;
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!(
                        "Winner ticket #{0} selected for epoch {1}",
                        winner_id,
                        pool.epoch_id,
                    ),
                )
            }),
        );
        Ok(())
    }
    /// Commit final state to L1 and undelegate the LotteryPool from the ER.
    pub fn undelegate_pool(ctx: Context<UndelegatePool>, _epoch_id: u64) -> Result<()> {
        if !(!ctx.accounts.lottery_pool.is_active) {
            return Err(
                anchor_lang::error::Error::from(anchor_lang::error::AnchorError {
                    error_name: LottryError::PoolStillActive.name(),
                    error_code_number: LottryError::PoolStillActive.into(),
                    error_msg: LottryError::PoolStillActive.to_string(),
                    error_origin: Some(
                        anchor_lang::error::ErrorOrigin::Source(anchor_lang::error::Source {
                            filename: "programs/lotry/src/lib.rs",
                            line: 282u32,
                        }),
                    ),
                    compared_values: None,
                }),
            );
        }
        commit_and_undelegate_accounts(
            &ctx.accounts.payer,
            <[_]>::into_vec(
                ::alloc::boxed::box_new([&ctx.accounts.lottery_pool.to_account_info()]),
            ),
            &ctx.accounts.magic_context,
            &ctx.accounts.magic_program,
        )?;
        ::solana_msg::sol_log(
            &::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!(
                        "LotteryPool epoch {0} committed & undelegated",
                        ctx.accounts.lottery_pool.epoch_id,
                    ),
                )
            }),
        );
        Ok(())
    }
    use ephemeral_rollups_sdk::cpi::undelegate_account;
    #[automatically_derived]
    pub fn process_undelegation(
        ctx: Context<InitializeAfterUndelegation>,
        account_seeds: Vec<Vec<u8>>,
    ) -> Result<()> {
        let [delegated_account, buffer, payer, system_program] = [
            &ctx.accounts.base_account,
            &ctx.accounts.buffer,
            &ctx.accounts.payer,
            &ctx.accounts.system_program,
        ];
        undelegate_account(
            delegated_account,
            &id(),
            buffer,
            payer,
            system_program,
            account_seeds,
        )?;
        Ok(())
    }
    #[automatically_derived]
    pub struct InitializeAfterUndelegation<'info> {
        /// CHECK:`
        #[account(mut)]
        pub base_account: AccountInfo<'info>,
        /// CHECK:`
        #[account()]
        pub buffer: AccountInfo<'info>,
        /// CHECK:`
        #[account(mut)]
        pub payer: AccountInfo<'info>,
        /// CHECK:`
        pub system_program: AccountInfo<'info>,
    }
    #[automatically_derived]
    impl<'info> anchor_lang::Accounts<'info, InitializeAfterUndelegationBumps>
    for InitializeAfterUndelegation<'info>
    where
        'info: 'info,
    {
        #[inline(never)]
        fn try_accounts(
            __program_id: &anchor_lang::solana_program::pubkey::Pubkey,
            __accounts: &mut &'info [anchor_lang::solana_program::account_info::AccountInfo<
                'info,
            >],
            __ix_data: &[u8],
            __bumps: &mut InitializeAfterUndelegationBumps,
            __reallocs: &mut std::collections::BTreeSet<
                anchor_lang::solana_program::pubkey::Pubkey,
            >,
        ) -> anchor_lang::Result<Self> {
            let base_account: AccountInfo = anchor_lang::Accounts::try_accounts(
                    __program_id,
                    __accounts,
                    __ix_data,
                    __bumps,
                    __reallocs,
                )
                .map_err(|e| e.with_account_name("base_account"))?;
            let buffer: AccountInfo = anchor_lang::Accounts::try_accounts(
                    __program_id,
                    __accounts,
                    __ix_data,
                    __bumps,
                    __reallocs,
                )
                .map_err(|e| e.with_account_name("buffer"))?;
            let payer: AccountInfo = anchor_lang::Accounts::try_accounts(
                    __program_id,
                    __accounts,
                    __ix_data,
                    __bumps,
                    __reallocs,
                )
                .map_err(|e| e.with_account_name("payer"))?;
            let system_program: AccountInfo = anchor_lang::Accounts::try_accounts(
                    __program_id,
                    __accounts,
                    __ix_data,
                    __bumps,
                    __reallocs,
                )
                .map_err(|e| e.with_account_name("system_program"))?;
            if !&base_account.is_writable {
                return Err(
                    anchor_lang::error::Error::from(
                            anchor_lang::error::ErrorCode::ConstraintMut,
                        )
                        .with_account_name("base_account"),
                );
            }
            if !&payer.is_writable {
                return Err(
                    anchor_lang::error::Error::from(
                            anchor_lang::error::ErrorCode::ConstraintMut,
                        )
                        .with_account_name("payer"),
                );
            }
            Ok(InitializeAfterUndelegation {
                base_account,
                buffer,
                payer,
                system_program,
            })
        }
    }
    #[automatically_derived]
    impl<'info> anchor_lang::ToAccountInfos<'info> for InitializeAfterUndelegation<'info>
    where
        'info: 'info,
    {
        fn to_account_infos(
            &self,
        ) -> Vec<anchor_lang::solana_program::account_info::AccountInfo<'info>> {
            let mut account_infos = ::alloc::vec::Vec::new();
            account_infos.extend(self.base_account.to_account_infos());
            account_infos.extend(self.buffer.to_account_infos());
            account_infos.extend(self.payer.to_account_infos());
            account_infos.extend(self.system_program.to_account_infos());
            account_infos
        }
    }
    #[automatically_derived]
    impl<'info> anchor_lang::ToAccountMetas for InitializeAfterUndelegation<'info> {
        fn to_account_metas(
            &self,
            is_signer: Option<bool>,
        ) -> Vec<anchor_lang::solana_program::instruction::AccountMeta> {
            let mut account_metas = ::alloc::vec::Vec::new();
            account_metas.extend(self.base_account.to_account_metas(None));
            account_metas.extend(self.buffer.to_account_metas(None));
            account_metas.extend(self.payer.to_account_metas(None));
            account_metas.extend(self.system_program.to_account_metas(None));
            account_metas
        }
    }
    #[automatically_derived]
    impl<'info> anchor_lang::AccountsExit<'info> for InitializeAfterUndelegation<'info>
    where
        'info: 'info,
    {
        fn exit(
            &self,
            program_id: &anchor_lang::solana_program::pubkey::Pubkey,
        ) -> anchor_lang::Result<()> {
            anchor_lang::AccountsExit::exit(&self.base_account, program_id)
                .map_err(|e| e.with_account_name("base_account"))?;
            anchor_lang::AccountsExit::exit(&self.payer, program_id)
                .map_err(|e| e.with_account_name("payer"))?;
            Ok(())
        }
    }
    pub struct InitializeAfterUndelegationBumps {}
    #[automatically_derived]
    impl ::core::fmt::Debug for InitializeAfterUndelegationBumps {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(f, "InitializeAfterUndelegationBumps")
        }
    }
    impl Default for InitializeAfterUndelegationBumps {
        fn default() -> Self {
            InitializeAfterUndelegationBumps {
            }
        }
    }
    impl<'info> anchor_lang::Bumps for InitializeAfterUndelegation<'info>
    where
        'info: 'info,
    {
        type Bumps = InitializeAfterUndelegationBumps;
    }
    /// An internal, Anchor generated module. This is used (as an
    /// implementation detail), to generate a struct for a given
    /// `#[derive(Accounts)]` implementation, where each field is a Pubkey,
    /// instead of an `AccountInfo`. This is useful for clients that want
    /// to generate a list of accounts, without explicitly knowing the
    /// order all the fields should be in.
    ///
    /// To access the struct in this module, one should use the sibling
    /// `accounts` module (also generated), which re-exports this.
    pub(crate) mod __client_accounts_initialize_after_undelegation {
        use super::*;
        use anchor_lang::prelude::borsh;
        /// Generated client accounts for [`InitializeAfterUndelegation`].
        pub struct InitializeAfterUndelegation {
            pub base_account: Pubkey,
            pub buffer: Pubkey,
            pub payer: Pubkey,
            pub system_program: Pubkey,
        }
        impl borsh::ser::BorshSerialize for InitializeAfterUndelegation
        where
            Pubkey: borsh::ser::BorshSerialize,
            Pubkey: borsh::ser::BorshSerialize,
            Pubkey: borsh::ser::BorshSerialize,
            Pubkey: borsh::ser::BorshSerialize,
        {
            fn serialize<W: borsh::maybestd::io::Write>(
                &self,
                writer: &mut W,
            ) -> ::core::result::Result<(), borsh::maybestd::io::Error> {
                borsh::BorshSerialize::serialize(&self.base_account, writer)?;
                borsh::BorshSerialize::serialize(&self.buffer, writer)?;
                borsh::BorshSerialize::serialize(&self.payer, writer)?;
                borsh::BorshSerialize::serialize(&self.system_program, writer)?;
                Ok(())
            }
        }
        #[automatically_derived]
        impl anchor_lang::ToAccountMetas for InitializeAfterUndelegation {
            fn to_account_metas(
                &self,
                is_signer: Option<bool>,
            ) -> Vec<anchor_lang::solana_program::instruction::AccountMeta> {
                let mut account_metas = ::alloc::vec::Vec::new();
                account_metas
                    .push(
                        anchor_lang::solana_program::instruction::AccountMeta::new(
                            self.base_account,
                            false,
                        ),
                    );
                account_metas
                    .push(
                        anchor_lang::solana_program::instruction::AccountMeta::new_readonly(
                            self.buffer,
                            false,
                        ),
                    );
                account_metas
                    .push(
                        anchor_lang::solana_program::instruction::AccountMeta::new(
                            self.payer,
                            false,
                        ),
                    );
                account_metas
                    .push(
                        anchor_lang::solana_program::instruction::AccountMeta::new_readonly(
                            self.system_program,
                            false,
                        ),
                    );
                account_metas
            }
        }
    }
    /// An internal, Anchor generated module. This is used (as an
    /// implementation detail), to generate a CPI struct for a given
    /// `#[derive(Accounts)]` implementation, where each field is an
    /// AccountInfo.
    ///
    /// To access the struct in this module, one should use the sibling
    /// [`cpi::accounts`] module (also generated), which re-exports this.
    pub(crate) mod __cpi_client_accounts_initialize_after_undelegation {
        use super::*;
        /// Generated CPI struct of the accounts for [`InitializeAfterUndelegation`].
        pub struct InitializeAfterUndelegation<'info> {
            pub base_account: anchor_lang::solana_program::account_info::AccountInfo<
                'info,
            >,
            pub buffer: anchor_lang::solana_program::account_info::AccountInfo<'info>,
            pub payer: anchor_lang::solana_program::account_info::AccountInfo<'info>,
            pub system_program: anchor_lang::solana_program::account_info::AccountInfo<
                'info,
            >,
        }
        #[automatically_derived]
        impl<'info> anchor_lang::ToAccountMetas for InitializeAfterUndelegation<'info> {
            fn to_account_metas(
                &self,
                is_signer: Option<bool>,
            ) -> Vec<anchor_lang::solana_program::instruction::AccountMeta> {
                let mut account_metas = ::alloc::vec::Vec::new();
                account_metas
                    .push(
                        anchor_lang::solana_program::instruction::AccountMeta::new(
                            anchor_lang::Key::key(&self.base_account),
                            false,
                        ),
                    );
                account_metas
                    .push(
                        anchor_lang::solana_program::instruction::AccountMeta::new_readonly(
                            anchor_lang::Key::key(&self.buffer),
                            false,
                        ),
                    );
                account_metas
                    .push(
                        anchor_lang::solana_program::instruction::AccountMeta::new(
                            anchor_lang::Key::key(&self.payer),
                            false,
                        ),
                    );
                account_metas
                    .push(
                        anchor_lang::solana_program::instruction::AccountMeta::new_readonly(
                            anchor_lang::Key::key(&self.system_program),
                            false,
                        ),
                    );
                account_metas
            }
        }
        #[automatically_derived]
        impl<'info> anchor_lang::ToAccountInfos<'info>
        for InitializeAfterUndelegation<'info> {
            fn to_account_infos(
                &self,
            ) -> Vec<anchor_lang::solana_program::account_info::AccountInfo<'info>> {
                let mut account_infos = ::alloc::vec::Vec::new();
                account_infos
                    .extend(
                        anchor_lang::ToAccountInfos::to_account_infos(&self.base_account),
                    );
                account_infos
                    .extend(anchor_lang::ToAccountInfos::to_account_infos(&self.buffer));
                account_infos
                    .extend(anchor_lang::ToAccountInfos::to_account_infos(&self.payer));
                account_infos
                    .extend(
                        anchor_lang::ToAccountInfos::to_account_infos(
                            &self.system_program,
                        ),
                    );
                account_infos
            }
        }
    }
}
