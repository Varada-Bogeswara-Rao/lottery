import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Lotry } from "../target/types/lotry";
import { expect } from "chai";
import { PublicKey, Keypair } from "@solana/web3.js";
import BN from "bn.js";
import { randomBytes, createHash } from "crypto";

describe("lotry", () => {
  // Connections: use MagicBlock's devnet RPC for both ER + L1
  const rpcEndpoint = process.env.ANCHOR_PROVIDER_URL ?? "https://rpc.magicblock.app/devnet/";
  const teeConnection = new anchor.web3.Connection(rpcEndpoint, "confirmed");
  const l1Connection = new anchor.web3.Connection(rpcEndpoint, "confirmed");

  const wallet = anchor.Wallet.local();
  const provider = new anchor.AnchorProvider(teeConnection, wallet, {
    preflightCommitment: "confirmed",
  });

  anchor.setProvider(provider);

  const program = anchor.workspace.Lotry as Program<Lotry>;
  // Hardcode an epoch ID for predictability in tests (bump if rerunning on devnet)
  const epochId = new BN(36); // bump if Devnet complains about reused PDAs

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
    expect(poolState.epochId.toNumber()).to.equal(epochId.toNumber());
  });

  it("Phase 2: Delegate Lottery Pool to ER (Devnet)", async () => {
    const [poolPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("lottery_pool"), epochId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    // Ephemeral Rollup Delegation Program
    const DELEGATION_PROGRAM_ID = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");

    // The validator we want to delegate to (from default magicblock localnet)
    const TEE_VALIDATOR = new PublicKey("MUS3hc9TCw4cGC12vHNoYcCGzJG1txjgQLZWVoeNHNd");

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
        authority: provider.wallet.publicKey,
        validator: TEE_VALIDATOR,
        bufferLotteryPool: bufferPda,
        delegationRecordLotteryPool: delegationRecordPda,
        delegationMetadataLotteryPool: delegationMetadataPda,
        delegationProgram: DELEGATION_PROGRAM_ID,
        ownerProgram: program.programId,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
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

    // Issue on L1 devnet to mirror real session flow
    await program.methods
      .issueSession(ephemeral.publicKey, new BN(validUntil))
      .accounts({
        authority: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .rpc();

    const sessionAccountInfo = await l1Connection.getAccountInfo(sessionPda);
    const sessionState: any = program.coder.accounts.decode("sessionToken", sessionAccountInfo!.data);
    expect(sessionState.authority.toBase58()).to.equal(provider.wallet.publicKey.toBase58());
    expect(sessionState.ephemeralKey.toBase58()).to.equal(ephemeral.publicKey.toBase58());
    expect(sessionState.validUntil.toNumber()).to.be.closeTo(validUntil, 5);
  });

  it("Phase 4: Buy Ticket via Session Key on ER (Devnet)", async () => {
    const [poolPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("lottery_pool"), epochId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    // Fresh session for this phase
    const sessionKey = Keypair.generate();
    const validUntil = Math.floor(Date.now() / 1000) + 3600;
    const [sessionPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("session"), wallet.publicKey.toBuffer(), sessionKey.publicKey.toBuffer()],
      program.programId
    );

    await program.methods
      .issueSession(sessionKey.publicKey, new BN(validUntil))
      .accounts({
        authority: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .rpc();

    // Read pool state from ER to derive deterministic ticket PDA.
    // We decode manually to avoid owner checks after delegation.
    const poolAccountInfo = await provider.connection.getAccountInfo(poolPda);
    if (!poolAccountInfo) {
      throw new Error("LotteryPool account not found on connection");
    }
    const poolState: any = program.coder.accounts.decode("lotteryPool", poolAccountInfo.data);
    const currentTicketCount = poolState.ticketCount.toNumber();

    const [playerTicketPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("player_ticket"),
        epochId.toArrayLike(Buffer, "le", 8),
        new BN(currentTicketCount).toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    const ticketData = Array.from(randomBytes(32));

    // Pre-allocate the PDA on L1 so the ER doesn't have to CPI to SystemProgram
    await program.methods
      .initPlayerTicket(epochId, new BN(currentTicketCount))
      .accounts({
        playerTicket: playerTicketPda,
        feePayer: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .rpc();

    // Ephemeral Rollup Delegation Program
    const DELEGATION_PROGRAM_ID = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
    const TEE_VALIDATOR = new PublicKey("MUS3hc9TCw4cGC12vHNoYcCGzJG1txjgQLZWVoeNHNd");

    const [ticketBufferPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("buffer"), playerTicketPda.toBuffer()],
      program.programId
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
    const ticketDelegateIx = await program.methods
      .delegatePlayerTicket(epochId, new BN(currentTicketCount))
      .accounts({
        playerTicket: playerTicketPda,
        feePayer: provider.wallet.publicKey,
        validator: TEE_VALIDATOR,
        bufferPlayerTicket: ticketBufferPda,
        delegationRecord: ticketDelegationRecordPda,
        delegationMetadata: ticketDelegationMetadataPda,
        ephemeralRollupsProgram: DELEGATION_PROGRAM_ID,
        ownerProgram: program.programId,
        systemProgram: anchor.web3.SystemProgram.programId,
      } as any)
      .instruction();

    const ticketDelegateTx = new anchor.web3.Transaction().add(ticketDelegateIx);
    ticketDelegateTx.recentBlockhash = (await provider.connection.getLatestBlockhash()).blockhash;
    ticketDelegateTx.feePayer = provider.wallet.publicKey;

    await anchor.web3.sendAndConfirmTransaction(provider.connection, ticketDelegateTx, [
      (wallet as any).payer ?? wallet,
    ], { commitment: "confirmed" });

    const discriminatorBuffer = createHash('sha256').update('global:buy_ticket').digest();
    const discriminator = discriminatorBuffer.subarray(0, 8);

    // ABI: epoch_id (u64), ticket_count (u64), ticket_data ([u8; 32])
    const dataBuffer = Buffer.alloc(8 + 8 + 8 + 32);
    discriminator.copy(dataBuffer, 0);
    epochId.toArrayLike(Buffer, "le", 8).copy(dataBuffer, 8);
    new BN(currentTicketCount).toArrayLike(Buffer, "le", 8).copy(dataBuffer, 16);
    Buffer.from(ticketData).copy(dataBuffer, 24);

    const ix = new anchor.web3.TransactionInstruction({
      programId: program.programId,
      keys: [
        { pubkey: poolPda, isSigner: false, isWritable: true },
        { pubkey: playerTicketPda, isSigner: false, isWritable: true },
        { pubkey: provider.wallet.publicKey, isSigner: false, isWritable: false },
        { pubkey: sessionPda, isSigner: false, isWritable: true },
        { pubkey: sessionKey.publicKey, isSigner: true, isWritable: false },
        { pubkey: provider.wallet.publicKey, isSigner: true, isWritable: true },
        { pubkey: anchor.web3.SystemProgram.programId, isSigner: false, isWritable: false },
      ],
      data: dataBuffer
    });

    const tx = new anchor.web3.Transaction().add(ix);
    tx.recentBlockhash = (await provider.connection.getLatestBlockhash()).blockhash;
    tx.feePayer = wallet.publicKey;

    await anchor.web3.sendAndConfirmTransaction(provider.connection, tx, [
      (wallet as any).payer ?? wallet,
      sessionKey,
    ]);

    const ticketAccountInfo = await provider.connection.getAccountInfo(playerTicketPda);
    const ticketState: any = program.coder.accounts.decode("playerTicket", ticketAccountInfo!.data);
    expect(ticketState.owner.toBase58()).to.equal(wallet.publicKey.toBase58());
    expect(ticketState.epochId.toNumber()).to.equal(epochId.toNumber());
    expect(ticketState.ticketId.toNumber()).to.equal(currentTicketCount);

    const updatedPoolInfo = await provider.connection.getAccountInfo(poolPda);
    const updatedPool: any = program.coder.accounts.decode("lotteryPool", updatedPoolInfo!.data);
    expect(updatedPool.ticketCount.toNumber()).to.equal(currentTicketCount + 1);
  });
});
