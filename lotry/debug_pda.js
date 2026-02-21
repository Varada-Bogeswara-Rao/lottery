const { PublicKey } = require("@solana/web3.js");
const BN = require("bn.js");

const programId = new PublicKey("8EfoffNAfiKmbLZYJ6N6YvF7PmRmrfJHoPzGH5jh5jvW");
const epochId = new BN(8);

const [poolPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("lottery_pool"), epochId.toArrayLike(Buffer, "le", 8)],
    programId
);

console.log(poolPda.toBase58());
