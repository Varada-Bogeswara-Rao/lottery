const { Connection, PublicKey } = require("@solana/web3.js");
const BN = require("bn.js");

async function check() {
    const connection = new Connection("https://rpc.magicblock.app/devnet/", "confirmed");
    const programId = new PublicKey("6uuK1kSc5UtnDy7MzhztXQ5fPz3LA6GLwFxxTUvQzC6L");
    const delegationProgramId = new PublicKey("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
    const epochId = new BN(41);
    const ticketCount = new BN(0);

    const [poolPda] = PublicKey.findProgramAddressSync(
        [Buffer.from("lottery_pool"), epochId.toArrayLike(Buffer, "le", 8)],
        programId
    );

    const [ticketPda] = PublicKey.findProgramAddressSync(
        [Buffer.from("player_ticket"), epochId.toArrayLike(Buffer, "le", 8), ticketCount.toArrayLike(Buffer, "le", 8)],
        programId
    );

    const getValidator = async (pda) => {
        const [recordPda] = PublicKey.findProgramAddressSync(
            [Buffer.from("delegation"), pda.toBuffer()],
            delegationProgramId
        );
        const info = await connection.getAccountInfo(recordPda);
        if (!info) return "No Record Found";
        // The validator is at some offset. Let's look at the data.
        return info.data.toString("hex");
    };

    const poolInfo = await connection.getAccountInfo(poolPda);
    const ticketInfo = await connection.getAccountInfo(ticketPda);

    console.log("Pool PDA:", poolPda.toBase58());
    console.log("Pool Owner:", poolInfo ? poolInfo.owner.toBase58() : "Not Found");
    console.log("Pool Record Hex:", await getValidator(poolPda));

    console.log("Ticket PDA:", ticketPda.toBase58());
    console.log("Ticket Owner:", ticketInfo ? ticketInfo.owner.toBase58() : "Not Found");
    console.log("Ticket Record Hex:", await getValidator(ticketPda));
}

check();
