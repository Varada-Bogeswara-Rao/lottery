const { Connection, PublicKey } = require("@solana/web3.js");

async function check(url) {
    try {
        console.log("Checking:", url);
        const connection = new Connection(url, "confirmed");
        const bh = await connection.getLatestBlockhash();
        console.log("Success! Blockhash:", bh.blockhash);
    } catch (e) {
        console.error("Failed for", url, ":", e.message);
    }
}

async function run() {
    await check("https://rpc.magicblock.app/devnet/");
    await check("https://tee.magicblock.app");
}

run();
