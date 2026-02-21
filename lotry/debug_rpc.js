const { Connection, PublicKey } = require("@solana/web3.js");

async function run() {
    const url = "https://rpc.magicblock.app/devnet/";
    try {
        console.log("Connecting to:", url);
        const connection = new Connection(url, "confirmed");
        const balance = await connection.getBalance(new PublicKey("CdjFo9UW828ZMTkLiydwP9atPWuUTxPQkDBMiyEaJ2PR"));
        console.log("Balance:", balance);
    } catch (e) {
        console.error("Failed:", e);
    }
}

run();
