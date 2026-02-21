# MagicBlock Shielded Micro-Lotteries: Development Progress 

## Context for the Next Agent
You are picking up the development from **Phase 4: BuyTicket (TEE Execution)**. We are building a shielded micro-lottery application on Solana using the **MagicBlock Ephemeral Rollups (ER) SDK**. Do **NOT** use the Bolt ECS framework/macros for this; we are using standard Anchor programs annotated with `#[ephemeral]`, `#[delegate]`, etc. from the `ephemeral-rollups-sdk` (v0.6.5).

The current workspace is an Anchor project located at `/home/bunny/Lotry/lotry`.

---

## 🟢 What is DONE (Phases 1, 2, & 3)

### Phase 1: Core State & Initialization
- **State Accounts:** Defined `LotteryPool`, `PlayerTicket`, and `SessionToken` structs in `programs/lotry/src/lib.rs`.
- **Initialization:** Implemented the `initialize_lottery` instruction to allocate the `LotteryPool` PDA on L1.

### Phase 2: Delegation to Ephemeral Rollup
- **Instruction:** Implemented `delegate_lottery` using the `ephemeral-rollups-sdk`. This bundles the `LotteryPool` account to a MagicBlock TEE Validator (`FnE6VJT5QNZdedZPnCoLsARgBwoE6DeJNjBs2H1gySXA`).
- **Internal PDAs:** Successfully derived and passed the required internal Delegation Program PDAs using tags `b"buffer"`, `b"delegation"`, and `b"delegation-metadata"`.

### Phase 3: Session Key Issuance
- **Instruction:** Implemented `issue_session` properly on L1.
- **Testing:** Phase 3 passes perfectly! The script properly generated a session PDA containing a 1-hour expiration timestamp and an `ephemeral_key` authority.

*Note: We have been incrementally changing the `epochId` variable in `tests/lotry.ts` manually to avoid "account already in use" errors during back-to-back Devnet test runs.*

---

## 🟡 What needs to be done NEXT (Phase 4)

Your immediate goal is to implement and test **Phase 4: BuyTicket (TEE Execution)**. This step is critical because it runs inside the Ephemeral Rollup (ER) rather than L1.

### 1. Smart Contract Implementation (`programs/lotry/src/lib.rs`)
- Add the `buy_ticket` Anchor context and instruction handler.
- **Logic Required:**
  - Validate that the signer is the `ephemeral_key` stored in the `SessionToken` PDA. Throw `LottryError::InvalidSessionSigner` if not.
  - Validate that the `SessionToken` has not expired (`valid_until > Clock::get()?.unix_timestamp`). Throw `LottryError::SessionExpired` if expired.
  - Validate that the `LotteryPool` is `is_active` and the `epoch_id` matches.
  - **Action:** Increment `LotteryPool.ticket_count` by 1.
  - **Action:** Initialize a new `PlayerTicket` PDA. 
    - Seeds: `[b"ticket", lottery_pool.key().as_ref(), pool.ticket_count.to_le_bytes().as_ref()]`.
    - Fields: Set `owner` to the original L1 `authority` from the session token, set `ticket_data` to a hashed 32-byte guess, and set `ticket_id` to the current `ticket_count`.
- **Important:** Do *not* include a `system_program::transfer` for a ticket fee inside this handler. ER transactions are gasless and cannot perform cross-program invocations to move real L1 SOL while delegated. The fee logic defaults to 0 for this test phase.

### 2. TypeScript Integration Test (`tests/lotry.ts`)
- Add a new Mocha `it()` block: `"Phase 4: Buy Ticket on ER Validator (TEE)"`.
- **CRITICAL - RPC Routing:** For this transaction, you MUST send the transaction to the Ephemeral Rollup RPC validator, *NOT* L1 Devnet. 
  - ER RPC URL: `https://tee.magicblock.app`
  - Create a new `Connection` and new `AnchorProvider` pointing to this ER URL. instantiate a secondary `erProgram` handle using this provider.
- Derive the `PlayerTicket` PDA using the ticket counter (guess `0` for the first ticket).
- **CRITICAL - Signers:** Send the `buyTicket` transaction using *only* the `ephemeral` `Keypair` from Phase 3 as the signer. Do not sign with the main wallet. (Gasless!)
- Use `.rpc()` via the `erProgram` handle to dispatch the transaction.
- After the transaction settles, fetch the `LotteryPool` *from the ER RPC* and assert `ticket_count === 1`. 

### Summary Steps for You (Codex):
1. Write the `buy_ticket` Anchor handler in `lib.rs`.
2. Write the Phase 4 automated test in `lotry.ts`.
3. Test locally using `ANCHOR_PROVIDER_URL=https://rpc.magicblock.app/devnet/ yarn run ts-mocha -p ./tsconfig.json -t 1000000 tests/**/*.ts`
