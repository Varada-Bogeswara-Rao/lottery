import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Lotry } from "../target/types/lotry";
import { expect } from "chai";
import { PublicKey, Keypair } from "@solana/web3.js";
import BN from "bn.js";

describe("lotry", () => {
  // Connections: tee endpoint for rollup-executed txs, devnet L1 for airdrops/account checks
  const teeRpc = process.env.ANCHOR_PROVIDER_URL ?? "https://tee.magicblock.app";
  const teeConnection = new anchor.web3.Connection(teeRpc, "confirmed");
  const l1Connection = new anchor.web3.Connection("https://rpc.magicblock.app/devnet/", "confirmed");

  const wallet = anchor.Wallet.local();
  const provider = new anchor.AnchorProvider(teeConnection, wallet, {
    preflightCommitment: "confirmed",
  });

  anchor.setProvider(provider);

  const program = anchor.workspace.Lotry as Program<Lotry>;
  const epochId = new BN(9); // bump if Devnet complains about reused PDAs

  // Ensure the test wallet has funds on devnet L1
  // (Verified manually via debug script: 8.41 SOL)
  //   before(async () => {
  // ... skipping balance check
  //   });

  /*
   * Phase 1: Initialize the Lottery Pool on Devnet L1
  */

  it("Phase 1: Initialize Lottery Pool (Devnet)", async () => {
    const [poolPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("lottery_pool"), epochId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    await program.methods
      .initializeLottery(epochId)
      .accounts({
        authority: provider.wallet.publicKey,
      })
      .rpc();

    const poolState = await program.account.lotteryPool.fetch(poolPda);
    expect(poolState.epochId.toNumber()).to.equal(9);
  });

  it("Phase 2: Delegate Lottery Pool to ER (Devnet)", async () => {
    const [poolPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("lottery_pool"), epochId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    // Ephemeral Rollup Delegation Program
    const DELEGATION_PROGRAM_ID = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");

    // The validator we want to delegate to (from default magicblock localnet)
    const TEE_VALIDATOR = new PublicKey("FnE6VJT5QNZdedZPnCoLsARgBwoE6DeJNjBs2H1gySXA");

    // Deriving the PDA accounts required for the manual delegation
    const [bufferPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("buffer"), poolPda.toBuffer()],
      program.programId
    );

    const [delegationRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("delegation"), poolPda.toBuffer()],
      DELEGATION_PROGRAM_ID
    );

    const [delegationMetadataPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("delegation-metadata"), poolPda.toBuffer()],
      DELEGATION_PROGRAM_ID
    );

    await program.methods
      .delegateLottery(epochId)
      .accounts({
        lotteryPool: poolPda,
        authority: provider.wallet.publicKey,
        validator: TEE_VALIDATOR,
        bufferLotteryPool: bufferPda,
        delegationRecordLotteryPool: delegationRecordPda,
        delegationMetadataLotteryPool: delegationMetadataPda,
        delegationProgram: DELEGATION_PROGRAM_ID,
        ownerProgram: program.programId,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      // Use sendAndConfirmTransaction because .rpc() throws a cryptic error in the Anchor client
      // for this specific CPI setup
      .transaction()
      .then(async (tx) => {
        tx.recentBlockhash = (await l1Connection.getLatestBlockhash()).blockhash;
        tx.feePayer = wallet.publicKey;
        return await anchor.web3.sendAndConfirmTransaction(l1Connection, tx, [(wallet as any).payer ?? wallet]);
      });

    // Verification: Re-fetch the account info and check the owner has changed to DELEGATION_PROGRAM_ID
    const poolAccountInfo = await l1Connection.getAccountInfo(poolPda);
    expect(poolAccountInfo?.owner.toBase58()).to.equal(DELEGATION_PROGRAM_ID.toBase58());
  });

  it("Phase 3: Issue Session Key (Devnet)", async () => {
    const ephemeral = Keypair.generate();
    const validUntil = Math.floor(Date.now() / 1000) + 3600; // 1 hour from now

    const [sessionPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("session"), wallet.publicKey.toBuffer(), ephemeral.publicKey.toBuffer()],
      program.programId
    );

    await program.methods
      .issueSession(ephemeral.publicKey, new BN(validUntil))
      .accounts({
        sessionToken: sessionPda,
        authority: provider.wallet.publicKey,
      })
      .rpc();

    const sessionState = await program.account.sessionToken.fetch(sessionPda);
    expect(sessionState.authority.toBase58()).to.equal(provider.wallet.publicKey.toBase58());
    expect(sessionState.ephemeralKey.toBase58()).to.equal(ephemeral.publicKey.toBase58());
    expect(sessionState.validUntil.toNumber()).to.be.closeTo(validUntil, 5);
  });
});
