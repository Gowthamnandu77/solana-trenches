const anchor = require("@coral-xyz/anchor");
const { Connection, Keypair, PublicKey } = require("@solana/web3.js");
const fs = require("fs");

const RPC_URL = "https://api.devnet.solana.com";

const PROGRAM_ID = new PublicKey(
  "FA4U6fpdyYZPqYwnBv9NQWPu8Q53LTjG1iMr3HUtCw7d"
);

const WALLET_PATH =
  "/home/cure/.config/solana/devnet-dev-wallet.json";

const IDL_PATH =
  "/home/cure/solana-trenches/target/idl/solana_trenches.json";

async function main() {
  const connection = new Connection(RPC_URL, "confirmed");

  const secretKey = Uint8Array.from(
    JSON.parse(fs.readFileSync(WALLET_PATH, "utf8"))
  );

  const keypair = Keypair.fromSecretKey(secretKey);

  const wallet = new anchor.Wallet(keypair);

  const provider = new anchor.AnchorProvider(
    connection,
    wallet,
    { commitment: "confirmed" }
  );

  anchor.setProvider(provider);

  const idl = JSON.parse(fs.readFileSync(IDL_PATH, "utf8"));

  const program = new anchor.Program(idl, provider);

  console.log("Wallet:", keypair.publicKey.toBase58());
  console.log("Program:", PROGRAM_ID.toBase58());

  const balance = await connection.getBalance(keypair.publicKey);

  console.log(
    "Balance:",
    balance / anchor.web3.LAMPORTS_PER_SOL,
    "SOL"
  );

  const [counterPda, bump] = PublicKey.findProgramAddressSync(
    [Buffer.from("counter")],
    PROGRAM_ID
  );

  console.log("Counter PDA:", counterPda.toBase58());
  console.log("PDA bump:", bump);

  console.log("\nAnchor client loaded successfully.");
  console.log("Available instructions:");

  for (const instruction of idl.instructions) {
    console.log("-", instruction.name);
  }
}

main().catch((err) => {
  console.error("\nERROR:");
  console.error(err);
  process.exit(1);
});
