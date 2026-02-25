const { Connection, PublicKey } = require("@solana/web3.js");
(async () => {
    const l1 = new Connection("https://api.devnet.solana.com");
    const er = new Connection("https://devnet-as.magicblock.app/");
    const magicContext = new PublicKey("MaGiCHoWBX1P2pXyosUoAR3MoxG3C79X8wW3a2x8r6H");

    async function retry(fn) {
        for (let i = 0; i < 5; i++) {
            try { return await fn(); } catch (e) { }
        }
    }
    const l1Info = await retry(() => l1.getAccountInfo(magicContext));
    console.log("L1 Info:", l1Info ? "Exists" : "Null");

    const erInfo = await retry(() => er.getAccountInfo(magicContext));
    console.log("ER Info:", erInfo ? "Exists" : "Null");
})();
