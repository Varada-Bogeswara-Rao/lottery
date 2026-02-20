import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Lotry } from "../target/types/lotry";
import { assert } from "chai";
import { Keypair, PublicKey } from "@solana/web3.js";
import BN from "bn.js";

/**
 * Shielded Micro-Lotteries — Integration Tests
 *
 * Phase 1: Initialize LotteryPool on base layer (L1)
 * Phase 2: Delegate LotteryPool to ER (TEE validator)
 * Phase 3: Issue SessionToken
 * Phase 4: BuyTicket via session key (ER)
 * Phase 5: Request + Consume Randomness (VRF stub)
 * Phase 6: Undelegate and settle to L1
 */

describe("lotry", () => {
  // ── Provider / Program Setup ──────────────────────────────────────────────
  anchor.setProvider(anchor.AnchorProvider.env());
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const program = anchor.workspace.Lotry as Program<Lotry>;
  const authority = provider.wallet;

  // TEE validator pubkey (tee.magicblock.app)
  const TEE_VALIDATOR = new PublicKey(
    "FnE6VJT5QNZdedZPnCoLsARgBwoE6DeJNjBs2H1gySXA"
  );

  // Shared epoch ID for all phases
  const EPOCH_ID = new BN(1);

  // ── PDA helpers ───────────────────────────────────────────────────────────
  function findLotteryPool(epochId: BN): [PublicKey, number] {
    const epochBuf = Buffer.alloc(8);
    epochBuf.writeBigUInt64LE(BigInt(epochId.toString()));
    return PublicKey.findProgramAddressSync(
      [Buffer.from("lottery_pool"), epochBuf],
      program.programId
    );
  }

  function findSessionToken(
    authorityKey: PublicKey,
    ephemeralKey: PublicKey
  ): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
      [Buffer.from("session"), authorityKey.toBuffer(), ephemeralKey.toBuffer()],
      program.programId
    );
  }

  function findPlayerTicket(epochId: BN, ticketId: BN): [PublicKey, number] {
    const epochBuf = Buffer.alloc(8);
    epochBuf.writeBigUInt64LE(BigInt(epochId.toString()));
    const ticketBuf = Buffer.alloc(8);
    ticketBuf.writeBigUInt64LE(BigInt(ticketId.toString()));
    return PublicKey.findProgramAddressSync(
      [Buffer.from("player_ticket"), epochBuf, ticketBuf],
      program.programId
    );
  }

  // ── Phase 1: Initialize Lottery ───────────────────────────────────────────
  describe("Phase 1: Initialize Lottery", () => {
    it("creates LotteryPool with correct initial state", async () => {
      const [lotteryPoolPda] = findLotteryPool(EPOCH_ID);

      const tx = await program.methods
        .initializeLottery(EPOCH_ID)
        .accounts({
          lotteryPool: lotteryPoolPda,
          authority: authority.publicKey,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      console.log("Phase 1 tx:", tx);

      const pool = await program.account.lotteryPool.fetch(lotteryPoolPda);

      assert.ok(pool.authority.equals(authority.publicKey), "authority mismatch");
      assert.ok(pool.epochId.eq(EPOCH_ID), "epoch_id mismatch");
      assert.equal(pool.ticketCount.toNumber(), 0, "ticket_count should be 0");
      assert.equal(pool.totalFunds.toNumber(), 0, "total_funds should be 0");
      assert.isTrue(pool.isActive, "pool should be active");
      assert.isNull(pool.vrfRequestId, "vrf_request_id should be null");
      assert.isNull(pool.winnerTicketId, "winner_ticket_id should be null");

      console.log("✅ LotteryPool initialized:", pool);
    });
  });

  // ── Phase 2: Delegate Pool to ER ──────────────────────────────────────────
  // NOTE: This requires a live ER validator. In local testing, uses localnet ER
  // at localhost:7799 (programId: mAGicPQYBMvcYveUZA5F5UNNwyHvfYh5xkLS2Fr1mev)
  describe("Phase 2: Delegate Pool to ER", () => {
    it("delegates LotteryPool to TEE ER validator", async () => {
      const [lotteryPoolPda] = findLotteryPool(EPOCH_ID);

      const tx = await program.methods
        .delegatePool(EPOCH_ID)
        .accounts({
          lotteryPool: lotteryPoolPda,
          authority: authority.publicKey,
          validator: TEE_VALIDATOR,
        })
        .rpc();

      console.log("Phase 2 delegation tx:", tx);

      // After delegation, the account owner should be the delegation program
      const accountInfo = await provider.connection.getAccountInfo(lotteryPoolPda);
      assert.ok(accountInfo, "LotteryPool account should still exist");

      const DELEGATION_PROGRAM_ID = new PublicKey(
        "DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh"
      );
      assert.ok(
        accountInfo!.owner.equals(DELEGATION_PROGRAM_ID),
        `Account owner should be delegation program, got: ${accountInfo!.owner.toBase58()}`
      );

      console.log("✅ LotteryPool delegated to ER:", lotteryPoolPda.toBase58());
    });
  });

  // ── Phase 3: Issue Session Token ──────────────────────────────────────────
  describe("Phase 3: Issue Session Token", () => {
    // Ephemeral keypair simulates a client-side in-memory key
    const ephemeralKeypair = Keypair.generate();
    // Session valid for 1 hour from now
    const validUntil = new BN(Math.floor(Date.now() / 1000) + 3600);

    it("creates SessionToken with correct ephemeral_key and valid_until", async () => {
      const [sessionPda] = findSessionToken(
        authority.publicKey,
        ephemeralKeypair.publicKey
      );

      const tx = await program.methods
        .issueSession(ephemeralKeypair.publicKey, validUntil)
        .accounts({
          sessionToken: sessionPda,
          authority: authority.publicKey,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      console.log("Phase 3 session tx:", tx);

      const session = await program.account.sessionToken.fetch(sessionPda);

      assert.ok(
        session.authority.equals(authority.publicKey),
        "authority mismatch"
      );
      assert.ok(
        session.ephemeralKey.equals(ephemeralKeypair.publicKey),
        "ephemeral_key mismatch"
      );
      assert.ok(
        session.validUntil.eq(validUntil),
        "valid_until mismatch"
      );

      console.log("✅ SessionToken issued:", {
        pda: sessionPda.toBase58(),
        ephemeralKey: ephemeralKeypair.publicKey.toBase58(),
        validUntil: validUntil.toNumber(),
      });
    });
  });

  // ── Phase 4: Buy Ticket via session key ───────────────────────────────────
  // NOTE: In full integration test, the BuyTicket tx is routed to the ER RPC
  // and signed ONLY by the ephemeral keypair. Shown here as stub.
  describe("Phase 4: Buy Ticket", () => {
    it("increments ticket_count and creates PlayerTicket PDA", async () => {
      const ephemeralKeypair = Keypair.generate();
      const validUntil = new BN(Math.floor(Date.now() / 1000) + 3600);
      const ticketData = new Uint8Array(32).fill(0xab); // mock ticket data

      const [sessionPda] = findSessionToken(
        authority.publicKey,
        ephemeralKeypair.publicKey
      );
      const [lotteryPoolPda] = findLotteryPool(EPOCH_ID);
      const [playerTicketPda] = findPlayerTicket(EPOCH_ID, new BN(0));

      // First issue a fresh session
      await program.methods
        .issueSession(ephemeralKeypair.publicKey, validUntil)
        .accounts({
          sessionToken: sessionPda,
          authority: authority.publicKey,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      // Buy ticket — signed only by ephemeral key + fee payer
      const tx = await program.methods
        .buyTicket(EPOCH_ID, Array.from(ticketData))
        .accounts({
          lotteryPool: lotteryPoolPda,
          playerTicket: playerTicketPda,
          sessionToken: sessionPda,
          ephemeralSigner: ephemeralKeypair.publicKey,
          feePayer: authority.publicKey,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([ephemeralKeypair])
        .rpc();

      console.log("Phase 4 buy_ticket tx:", tx);

      const pool = await program.account.lotteryPool.fetch(lotteryPoolPda);
      assert.equal(pool.ticketCount.toNumber(), 1, "ticket_count should be 1");

      const ticket = await program.account.playerTicket.fetch(playerTicketPda);
      assert.ok(ticket.owner.equals(authority.publicKey), "owner mismatch");
      assert.equal(ticket.ticketId.toNumber(), 0, "ticket_id should be 0");

      console.log("✅ PlayerTicket created:", {
        pda: playerTicketPda.toBase58(),
        ticketId: ticket.ticketId.toNumber(),
        owner: ticket.owner.toBase58(),
      });
    });
  });

  // ── Phase 5: Request + Consume Randomness (stub) ──────────────────────────
  describe("Phase 5: Consume Randomness (stub)", () => {
    it("selects a winner via modulo on mock randomness and deactivates pool", async () => {
      const [lotteryPoolPda] = findLotteryPool(EPOCH_ID);

      // Mock 32-byte randomness (normally injected by VRF oracle)
      const mockRandomness = Buffer.alloc(32, 0x42); // 0x42 = ticket index 0

      // Stub oracle queue (any pubkey for the stub test)
      const stubOracleQueue = Keypair.generate().publicKey;
      const stubVrfIdentity = Keypair.generate();

      // request_winner (stub — doesn't actually CPI to VRF)
      await program.methods
        .requestWinner(EPOCH_ID, 42)
        .accounts({
          lotteryPool: lotteryPoolPda,
          payer: authority.publicKey,
          oracleQueue: stubOracleQueue,
        })
        .rpc();

      // consume_randomness (simulates oracle CPI callback)
      const tx = await program.methods
        .consumeRandomness(Array.from(mockRandomness))
        .accounts({
          vrfProgramIdentity: stubVrfIdentity.publicKey,
          lotteryPool: lotteryPoolPda,
        })
        .signers([stubVrfIdentity])
        .rpc();

      console.log("Phase 5 consume_randomness tx:", tx);

      const pool = await program.account.lotteryPool.fetch(lotteryPoolPda);
      assert.isFalse(pool.isActive, "pool should be inactive after draw");
      assert.isNotNull(pool.winnerTicketId, "winner_ticket_id should be set");

      const winnerTicketId = pool.winnerTicketId as BN;
      assert.ok(
        winnerTicketId.toNumber() >= 0 &&
        winnerTicketId.toNumber() < pool.ticketCount.toNumber(),
        `winner_ticket_id ${winnerTicketId.toNumber()} out of range [0, ${pool.ticketCount.toNumber()})`
      );

      console.log("✅ Winner selected:", winnerTicketId.toNumber());
    });
  });

  // ── Phase 6: Undelegate and Settle ────────────────────────────────────────
  // NOTE: This must be called on the ER RPC after all tickets purchased.
  // The commit_and_undelegate_accounts CPI sends state diffs back to L1.
  describe("Phase 6: Undelegate Pool", () => {
    it("commits final state to L1 and undelegates the LotteryPool", async () => {
      const [lotteryPoolPda] = findLotteryPool(EPOCH_ID);

      const tx = await program.methods
        .undelegatePool(EPOCH_ID)
        .accounts({
          lotteryPool: lotteryPoolPda,
          payer: authority.publicKey,
        })
        .rpc();

      console.log("Phase 6 undelegate tx:", tx);

      // After undelegation, fetch from the L1 RPC — should be owned by our program again
      const accountInfo = await provider.connection.getAccountInfo(lotteryPoolPda);
      assert.ok(accountInfo, "LotteryPool should still exist on L1 after settlement");
      assert.ok(
        accountInfo!.owner.equals(program.programId),
        `After undelegation, owner should be program: ${program.programId.toBase58()}, got: ${accountInfo!.owner.toBase58()}`
      );

      // Pool should be settled (inactive)
      const pool = await program.account.lotteryPool.fetch(lotteryPoolPda);
      assert.isFalse(pool.isActive, "pool should be inactive after settlement");

      console.log("✅ LotteryPool settled on L1:", {
        pda: lotteryPoolPda.toBase58(),
        owner: accountInfo!.owner.toBase58(),
        winnerTicketId: (pool.winnerTicketId as BN | null)?.toNumber(),
      });
    });
  });
});
