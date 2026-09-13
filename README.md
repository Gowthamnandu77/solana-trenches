# Solana Trenches

A hands-on Solana development project built with **Rust**, **Anchor**, and **JavaScript**.

This repository is the foundation for a larger Solana/Web3 engineering roadmap focused on real-time blockchain data, trading infrastructure, backtesting, automation, and production-grade systems.

## Current Status

The project currently includes:

- An Anchor smart contract written in Rust
- `initialize` and `increment` instructions
- PDA-based account handling
- A JavaScript client using `@coral-xyz/anchor`
- Devnet deployment
- Local testing with LiteSVM
- Git-based development and deployment workflow

## Program Information

**Network:** Solana Devnet

**Program ID:**

```text
FA4U6fpdyYZPqYwnBv9NQWPu8Q53LTjG1iMr3HUtCw7d
```

## Project Structure

```text
solana-trenches/
├── client/
│   ├── index.js
│   ├── package.json
│   └── package-lock.json
│
├── programs/
│   └── solana-trenches/
│       ├── Cargo.toml
│       ├── src/
│       │   ├── lib.rs
│       │   ├── constants.rs
│       │   ├── error.rs
│       │   ├── instructions.rs
│       │   ├── state.rs
│       │   └── instructions/
│       │       ├── initialize.rs
│       │       └── increment.rs
│       └── tests/
│
├── Anchor.toml
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
└── .gitignore
```

## Tech Stack

- Rust
- Solana
- Anchor Framework
- JavaScript / Node.js
- `@coral-xyz/anchor`
- `@solana/web3.js`
- Git / GitHub
- WSL Ubuntu

## Prerequisites

Install:

- Rust
- Solana CLI
- Anchor CLI
- Node.js
- npm

Check your installations:

```bash
rustc --version
cargo --version
solana --version
anchor --version
node --version
npm --version
```

## Setup

Clone the repository:

```bash
git clone https://github.com/Gowthamnandu77/solana-trenches.git
cd solana-trenches
```

Install client dependencies:

```bash
cd client
npm install
cd ..
```

## Solana Configuration

Check the current Solana configuration:

```bash
solana config get
```

For Devnet:

```bash
solana config set --url devnet
```

This repository does **not** include private wallet files, seed phrases, or private keys.

Use your own local Solana wallet when running the client or deploying programs.

## Build

```bash
anchor build
```

## Test

For the current test setup:

```bash
anchor test --skip-local-validator
```

## Deploy

Deploy to Devnet:

```bash
anchor deploy
```

Make sure your Solana CLI is configured for Devnet and your local wallet has enough Devnet SOL.

## Run the Client

From the repository root:

```bash
cd client
node index.js
```

The client can:

- Load the Anchor IDL
- Connect to Solana
- Load a local wallet
- Create an Anchor provider
- Access the deployed program
- Derive the program PDA
- Read wallet balance
- Inspect available program instructions

## Security

Sensitive files should never be committed.

The repository ignores files such as:

```text
target/
node_modules/
.env
*-wallet.json
*.keypair.json
```

Never commit:

- Seed phrases
- Private keys
- Wallet JSON files
- API keys
- RPC secrets
- Exchange credentials

## What I Learned

This project is being built as practical proof of work rather than as a tutorial-only exercise.

Topics covered so far include:

- Solana accounts and programs
- Anchor project structure
- Program IDs
- PDAs and bumps
- On-chain instructions
- Rust smart contract development
- Devnet deployment
- Client-to-program interaction
- Solana CLI configuration
- Git and GitHub workflow
- Basic smart contract testing

## Roadmap

Next stages:

- [ ] Real-time Solana RPC/WebSocket data ingestion
- [ ] Transaction and log parsing
- [ ] Token and DEX event filtering
- [ ] New-pair detection
- [ ] Market acceleration signals
- [ ] Structured event storage
- [ ] Paper trading engine
- [ ] Historical backtesting
- [ ] Risk management
- [ ] Production API and database layer
- [ ] Automated execution
- [ ] AI/ML-based market intelligence

## Long-Term Goal

The long-term goal is to evolve this repository into a real Solana market-intelligence and trading infrastructure project capable of:

```text
Solana Blockchain
        ↓
Real-Time Data Ingestion
        ↓
Transaction / DEX Parsing
        ↓
Market Intelligence
        ↓
Signal Engine
        ↓
Paper Trading
        ↓
Backtesting
        ↓
Risk Management
        ↓
Automated Execution
```

## Disclaimer

This project is for software engineering, blockchain development, research, and educational purposes.

Nothing in this repository should be considered financial advice.

## Author

**Gowtham**

GitHub: [@Gowthamnandu77](https://github.com/Gowthamnandu77)
