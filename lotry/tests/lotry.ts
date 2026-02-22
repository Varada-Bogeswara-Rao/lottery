import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { MAGIC_CONTEXT_ID, MAGIC_PROGRAM_ID } from "@magicblock-labs/ephemeral-rollups-sdk";
import { Lotry } from "../target/types/lotry";
import { expect } from "chai";
import { PublicKey, Keypair, SYSVAR_SLOT_HASHES_PUBKEY } from "@solana/web3.js";
import BN from "bn.js";
import { randomBytes, createHash } from "crypto";

describe("lotry", () => {
  // Connections: use MagicBlock's devnet RPC for both ER + L1
  // Connections: devnet-router for L1/Router, devnet-as for TEE ER
  const l1Endpoint = "https://api.devnet.solana.com";
  const erEndpoint = "https://devnet-as.magicblock.app/";
  const l1Connection = new anchor.web3.Connection(l1Endpoint, "confirmed");
  const erConnection = new anchor.web3.Connection(erEndpoint, "confirmed");

  const wallet = anchor.Wallet.local();
  const l1Provider = new anchor.AnchorProvider(l1Connection, wallet, { preflightCommitment: "confirmed" });
  const erProvider = new anchor.AnchorProvider(erConnection, wallet, { preflightCommitment: "confirmed" });

  const l1Program = new Program(anchor.workspace.Lotry.idl, l1Provider) as Program<Lotry>;
  const erProgram = new Program(anchor.workspace.Lotry.idl, erProvider) as Program<Lotry>;

  // Hardcode an epoch ID for predictability in tests (bump if rerunning on devnet)
  const epochId = new BN(1030); // bump if Devnet complains about reused PDAs
  const sessionKey = Keypair.generate();
  const validUntil = Math.floor(Date.now() / 1000) + 3600; // 1 hour from now

  const [poolPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("lottery_pool"), epochId.toArrayLike(Buffer, "le", 8)],
    l1Program.programId
  );
  const [sessionPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("session"), wallet.publicKey.toBuffer(), sessionKey.publicKey.toBuffer()],
    l1Program.programId
  );
  const [playerTicketPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("player_ticket"), epochId.toArrayLike(Buffer, "le", 8), new BN(0).toArrayLike(Buffer, "le", 8)],
    l1Program.programId
  );

  const withRetry = async <T>(fn: () => Promise<T>, retries = 5, delayMs = 5000): Promise<T> => {
    for (let i = 0; i < retries; i++) {
      try {
        return await fn();
      } catch (e) {
        if (i === retries - 1) throw e;
        console.log(`Retry ${i + 1}/${retries} after error: ${e.message}`);
        await new Promise(r => setTimeout(r, delayMs));
      }
    }
    throw new Error("Retry failed");
  };

  /*
   * Phase 1: Initialize the Lottery Pool on Devnet L1
  */

  it("Phase 1: Initialize Lottery Pool (Devnet)", async () => {

    await withRetry(() => l1Program.methods
      .initializeLottery(epochId)
      .accounts({
        authority: l1Provider.wallet.publicKey,
      })
      .rpc());

    console.log("Phase 1: Lottery Pool initialized for epoch", epochId.toNumber());

    const poolState = await withRetry(() => erProgram.account.lotteryPool.fetch(poolPda));
    expect(poolState.epochId.toNumber()).to.equal(epochId.toNumber());
  });

  it("Phase 2: Delegate Lottery Pool to ER (Devnet)", async () => {

    // Ephemeral Rollup Delegation Program
    const DELEGATION_PROGRAM_ID = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");

    // The validator we want to delegate to (from default magicblock localnet)
    const TEE_VALIDATOR = new PublicKey("MAS1Dt9qreoRMQ14YQuhg8UTZMMzDdKhmkZMECCzk57");

    // Deriving the PDA accounts required for the manual delegation
    const [bufferPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("buffer"), poolPda.toBuffer()],
      l1Program.programId
    );

    const [delegationRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("delegation"), poolPda.toBuffer()],
      DELEGATION_PROGRAM_ID
    );

    const [delegationMetadataPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("delegation-metadata"), poolPda.toBuffer()],
      DELEGATION_PROGRAM_ID
    );

    await withRetry(() => l1Program.methods
      .delegateLottery(epochId)
      .accounts({
        authority: l1Provider.wallet.publicKey,
        validator: TEE_VALIDATOR,
        bufferLotteryPool: bufferPda,
        delegationRecordLotteryPool: delegationRecordPda,
        delegationMetadataLotteryPool: delegationMetadataPda,
        delegationProgram: DELEGATION_PROGRAM_ID,
        ownerProgram: l1Program.programId,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .transaction()
      .then(async (tx) => {
        tx.recentBlockhash = (await l1Connection.getLatestBlockhash()).blockhash;
        tx.feePayer = wallet.publicKey;
        return await anchor.web3.sendAndConfirmTransaction(l1Connection, tx, [(wallet as any).payer ?? wallet]);
      }));

    // Verification: Re-fetch the account info and check the owner has changed to DELEGATION_PROGRAM_ID
    const poolAccountInfo = await withRetry(() => l1Connection.getAccountInfo(poolPda));
    expect(poolAccountInfo?.owner.toBase58()).to.equal(DELEGATION_PROGRAM_ID.toBase58());
  });

  it("Phase 3: Issue Session Key (Devnet)", async () => {
    // Issue on L1 devnet to mirror real session flow
    await withRetry(() => l1Program.methods
      .issueSession(sessionKey.publicKey, new BN(validUntil))
      .accounts({
        authority: l1Provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .rpc());

    const sessionAccountInfo = await withRetry(() => l1Connection.getAccountInfo(sessionPda));
    const sessionState: any = erProgram.coder.accounts.decode("sessionToken", sessionAccountInfo!.data);
    expect(sessionState.authority.toBase58()).to.equal(l1Provider.wallet.publicKey.toBase58());
    expect(sessionState.ephemeralKey.toBase58()).to.equal(sessionKey.publicKey.toBase58());
    expect(sessionState.validUntil.toNumber()).to.be.closeTo(validUntil, 5);
  });

  it("Phase 4: Buy Ticket via Session Key on ER (Devnet)", async () => {
    // Read pool state from ER to derive deterministic ticket PDA.
    // We decode manually to avoid owner checks after delegation.
    const poolAccountInfo = await withRetry(() => erConnection.getAccountInfo(poolPda));
    if (!poolAccountInfo) {
      throw new Error("LotteryPool account not found on connection");
    }
    const poolState: any = erProgram.coder.accounts.decode("lotteryPool", poolAccountInfo.data);
    const currentTicketCount = poolState.ticketCount.toNumber();

    // The playerTicketPda for ticket 0 is already derived at the top level
    // for this test epoch if we only buy 1 ticket.

    const ticketData = Array.from(randomBytes(32));

    // Pre-allocate the PDA on L1 so the ER doesn't have to CPI to SystemProgram
    await withRetry(() => l1Program.methods
      .initPlayerTicket(epochId, new BN(currentTicketCount))
      .accounts({
        playerTicket: playerTicketPda,
        feePayer: l1Provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .rpc());

    // Ephemeral Rollup Delegation Program
    const DELEGATION_PROGRAM_ID = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
    const TEE_VALIDATOR = new PublicKey("MAS1Dt9qreoRMQ14YQuhg8UTZMMzDdKhmkZMECCzk57");

    const [ticketBufferPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("buffer"), playerTicketPda.toBuffer()],
      l1Program.programId
    );
    const [ticketDelegationRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("delegation"), playerTicketPda.toBuffer()],
      DELEGATION_PROGRAM_ID
    );
    const [ticketDelegationMetadataPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("delegation-metadata"), playerTicketPda.toBuffer()],
      DELEGATION_PROGRAM_ID
    );

    // Delegate the PlayerTicket to the ER before modifying it inside the TEE
    const ticketDelegateIx = await l1Program.methods
      .delegatePlayerTicket(epochId, new BN(currentTicketCount))
      .accounts({
        playerTicket: playerTicketPda,
        feePayer: l1Provider.wallet.publicKey,
        validator: TEE_VALIDATOR,
        bufferPlayerTicket: ticketBufferPda,
        delegationRecord: ticketDelegationRecordPda,
        delegationMetadata: ticketDelegationMetadataPda,
        ephemeralRollupsProgram: DELEGATION_PROGRAM_ID,
        ownerProgram: l1Program.programId,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .instruction();

    const ticketDelegateTx = new anchor.web3.Transaction().add(ticketDelegateIx);
    ticketDelegateTx.recentBlockhash = (await l1Connection.getLatestBlockhash()).blockhash;
    ticketDelegateTx.feePayer = l1Provider.wallet.publicKey;

    await withRetry(() => anchor.web3.sendAndConfirmTransaction(l1Connection, ticketDelegateTx, [
      (wallet as any).payer ?? wallet,
    ], { commitment: "confirmed" }));

    const tx = await withRetry(() => erProgram.methods
      .buyTicket(epochId, new BN(currentTicketCount), Array.from(ticketData))
      .accounts({
        lotteryPool: poolPda,
        playerTicket: playerTicketPda,
        authority: wallet.publicKey,
        sessionToken: sessionPda,
        ephemeralSigner: sessionKey.publicKey,
        feePayer: wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .signers([sessionKey])
      .rpc());

    console.log("BuyTicket successful on ER! TX:", tx);

    const ticketAccountInfo = await withRetry(() => erConnection.getAccountInfo(playerTicketPda));
    const ticketState: any = erProgram.coder.accounts.decode("playerTicket", ticketAccountInfo!.data);
    expect(ticketState.owner.toBase58()).to.equal(wallet.publicKey.toBase58());
    expect(ticketState.epochId.toNumber()).to.equal(epochId.toNumber());
    expect(ticketState.ticketId.toNumber()).to.equal(currentTicketCount);

    const updatedPoolInfo = await withRetry(() => erConnection.getAccountInfo(poolPda));
    const updatedPool: any = erProgram.coder.accounts.decode("lotteryPool", updatedPoolInfo!.data);
    expect(updatedPool.ticketCount.toNumber()).to.equal(currentTicketCount + 1);
  });

  it("Phase 5: Request Winner via Session Key (Devnet)", async () => {
    const [poolPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("lottery_pool"), epochId.toArrayLike(Buffer, "le", 8)],
      l1Program.programId
    );

    // Simplified Phase 5: Request Winner directly (Step 3 Isolation)

    const poolAccountInfo = await withRetry(() => erConnection.getAccountInfo(poolPda));
    if (!poolAccountInfo) {
      throw new Error("LotteryPool account not found on connection");
    }
    const poolState: any = erProgram.coder.accounts.decode("lotteryPool", poolAccountInfo.data);
    const ticketCount = poolState.ticketCount.toNumber();
    expect(ticketCount).to.be.greaterThan(0);


    const clientSeed = 7;

    const requestTx = await withRetry(() => erProgram.methods
      .requestWinner(epochId, clientSeed)
      .accounts({
        lotteryPool: poolPda,
        authority: wallet.publicKey,
        sessionToken: sessionPda,
        ephemeralSigner: sessionKey.publicKey,
      } as any)
      .signers([sessionKey])
      .rpc());

    console.log("Winner selected on ER! TX:", requestTx);

    const finalPoolInfo = await withRetry(() => erConnection.getAccountInfo(poolPda));
    const finalPool: any = erProgram.coder.accounts.decode("lotteryPool", finalPoolInfo.data);

    console.log("\nWinner selected! Ticket ID:", finalPool.winnerTicketId.toNumber());
    expect(finalPool.isActive).to.equal(false);
    expect(finalPool.winnerTicketId).to.not.equal(null);
    expect(finalPool.winnerTicketId.toNumber()).to.be.at.least(0);
    expect(finalPool.winnerTicketId.toNumber()).to.be.lessThan(ticketCount);
  });

  it("Phase 6: Commit & Undelegate Lottery Pool (Devnet)", async () => {
    const commitTx = await withRetry(() => erProgram.methods
      .undelegatePool(epochId)
      .accounts({
        lotteryPool: poolPda,
        payer: wallet.publicKey,
        magicContext: MAGIC_CONTEXT_ID,
        magicProgram: MAGIC_PROGRAM_ID,
      } as any)
      .rpc());

    console.log("Commit + undelegate scheduled on ER! TX:", commitTx);

    const l1PoolState = await withRetry(async () => {
      const l1PoolInfo = await l1Connection.getAccountInfo(poolPda);
      if (!l1PoolInfo) {
        throw new Error("LotteryPool account not found on L1");
      }
      if (l1PoolInfo.owner.toBase58() !== l1Program.programId.toBase58()) {
        throw new Error("LotteryPool not yet undelegated on L1");
      }
      const state: any = l1Program.coder.accounts.decode("lotteryPool", l1PoolInfo.data);
      if (state.isActive) {
        throw new Error("LotteryPool still active on L1");
      }
      if (state.winnerTicketId === null) {
        throw new Error("Winner not committed on L1");
      }
      return state;
    });

    console.log("LotteryPool committed on L1. Winner ticket:", l1PoolState.winnerTicketId.toNumber());
    expect(l1PoolState.isActive).to.equal(false);
    expect(l1PoolState.winnerTicketId.toNumber()).to.be.at.least(0);
  });
});
