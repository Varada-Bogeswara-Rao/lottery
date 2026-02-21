# Required Changes for Phase 4 Debugging

## 1. Rust Smart Contract (`lib.rs`)

### Fix Cyclic PDA Dependency & IDL Crash
The `BuyTicket` struct had a cyclic dependency: the `session_token` PDA seeds relied on `session_token.authority`. This causes the TypeScript client to crash with `TypeError: Cannot read properties of undefined (reading 'size')` when loading the IDL.

**Fix:**
* Add `pub authority: UncheckedAccount<'info>` to the `BuyTicket` struct.
* Update `session_token` seeds to use `authority.key().as_ref()` instead of `session_token.authority`.
* Add `has_one = authority` constraint to `session_token` for safety.

### Fix Macro Expansion Conflict
The `#[ephemeral]` attribute on individual functions can interfere with Anchor 0.30+ internal macros. 

**Fix:**
* Use `#[ephemeral]` at the **module level** (above `#[program]`). This activates Bolt's ephemeral tier features for the entire module safely.
* If a function-level attribute is still desired, use the fully qualified path: `#[ephemeral_rollups_sdk::anchor::ephemeral]`.

---

## 3. Resolving Runtime Errors

### Fix Error 3007 (AccountNotInitialized) in Phase 4
If `buy_ticket` throws 3007, it means the `session_token` account exists on Devnet L1 but is not visible to the Ephemeral Rollup.

**Fix:**
* Ensure `issue_session` is also marked as an ephemeral instruction (either via the module-level attribute or specifically) so the session PDA is created directly on the ER.
* Verify that the `authority` passed to `buy_ticket` matches exactly the one used to derive the session on ER.

### Fix Unknown action 'undefined'
This error occurs in Anchor 0.32.1 when using `.rpc()` on ephemeral instructions because the RPC response format from the TEE might slightly differ from standard L1 Solana, triggering a bug in Anchor's provider.

**Fix:**
* Use `.transaction()` instead of `.rpc()` for `buyTicket`.
* Manually sign the transaction with the session key and the wallet.
* Use `anchor.web3.sendAndConfirmTransaction` to submit.

Example:
```typescript
const tx = await program.methods.buyTicket(...).accounts({...} as any).transaction();
tx.recentBlockhash = (await provider.connection.getLatestBlockhash()).blockhash;
tx.feePayer = provider.wallet.publicKey;
await anchor.web3.sendAndConfirmTransaction(provider.connection, tx, [wallet.payer, sessionKey]);
```

---

## 4. Final Verification Checklist
1. `epochId` is bumped (e.g., to 21).
2. `BuyTicket` struct in Rust has `authority: UncheckedAccount`.
3. `session_token` seeds use `authority.key()`.
4. `#[ephemeral]` is at the module level.
5. `lotry.ts` uses manual transaction submission for Phase 4.

---

## 2. TypeScript Tests (`lotry.ts`)

### Fix "Maximum Depth" / Auto-Resolution Error
Anchor's auto-resolver (v0.30+) tries to `.fetch()` accounts to derive PDA seeds. Since `lotteryPool` is delegated to the MagicBlock program, the fetch fails with an ownership mismatch.

**Fix:**
* In Phase 4 `buyTicket.accounts()`, manually pass the `lotteryPool` and `playerTicket` PDAs.
* Cast the accounts object to `any` (e.g., `.accounts({ ... } as any)`) to bypass the strict IDL check and silence the auto-resolver.

### Fix RPC Routing
The explicit `https://tee.magicblock.app` endpoint returns 500 errors without a token query param.

**Fix:**
* Use `https://rpc.magicblock.app/devnet/` for all connections. MagicBlock's RPC automatically routes transactions to the correct TEE validator based on the accounts involved.

### Fix Phase 1 Assertion
The assertion `expect(poolState.epochId.toNumber()).to.equal(13)` was hardcoded.

**Fix:**
* Change to `expect(poolState.epochId.toNumber()).to.equal(epochId.toNumber())`.

---

## Summary of what was done
- Identified PDAs seeds mismatch in Phase 2 (`delegate_buffer` -> `buffer`, etc).
- Identified cyclic dependency in `BuyTicket` struct causing TS crashes.
- Identified `#[ephemeral]` macro placement issue.
- Fixed Anchor auto-resolution bugs by switching to manual account passing for delegated state.
