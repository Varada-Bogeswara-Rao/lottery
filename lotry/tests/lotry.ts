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

  // Ensure the test wallet has funds on devnet L1
  // (Verified manually via debug script: 8.41 SOL)
  /*
  before(async () => {
    const bal = await l1Connection.getBalance(wallet.publicKey);
    if (bal < anchor.web3.LAMPORTS_PER_SOL) {
      const sig = await l1Connection.requestAirdrop(
        wallet.publicKey,
        2 * anchor.web3.LAMPORTS_PER_SOL
      );
      await l1Connection.confirmTransaction(sig, "confirmed");
    }
  });
  */

  it("Phase 1: Initialize Lottery Pool (Devnet)", async () => {
    const epochId = new BN(8); // Increment to avoid already-in-use error on Devnet

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
    expect(poolState.epochId.toNumber()).to.equal(8);
  });

  it("Phase 2: Delegate Lottery Pool to ER (Devnet)", async () => {
    const epochId = new BN(8);

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

    const accounts: any = {
      lotteryPool: poolPda,
      authority: provider.wallet.publicKey,
      validator: TEE_VALIDATOR,
      bufferLotteryPool: bufferPda,
      delegationRecordLotteryPool: delegationRecordPda,
      delegationMetadataLotteryPool: delegationMetadataPda,
      delegationProgram: DELEGATION_PROGRAM_ID,
      ownerProgram: program.programId,
      systemProgram: anchor.web3.SystemProgram.programId,
    };

    try {
      // Manual transaction to bypass .rpc() issue
      const tx = await program.methods
        .delegateLottery(epochId)
        .accounts(accounts)
        .transaction();

      const sig = await anchor.web3.sendAndConfirmTransaction(
        l1Connection,
        tx,
        [(wallet as any).payer ?? wallet] // Support different wallet structures
      );
      console.log("Delegation Sig:", sig);
    } catch (err: any) {
      if (err.logs) {
        console.log("Transaction Logs:", err.logs);
      }
      throw err;
    }

    // Verify ownership has changed to the delegation program
    const poolAccountInfo = await l1Connection.getAccountInfo(poolPda);
    expect(poolAccountInfo?.owner.toBase58()).to.equal(DELEGATION_PROGRAM_ID.toBase58());
  });
});

