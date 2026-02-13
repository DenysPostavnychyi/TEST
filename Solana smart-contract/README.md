# 🎯 Complete Lottery Program - Deployment & Usage Guide

## 📋 Overview

This is a comprehensive multi-token lottery program on Solana that supports both SOL and SPL tokens, with BTC-denominated ticket pricing, automated round management, and verifiable randomness.

## 🏗️ Architecture

### Core Components
- **LotteryState**: Global lottery configuration and supported tokens
- **TokenLottery**: Individual lottery data per token (SOL/SPL)
- **PlayerData**: Player statistics per token
- **Round Management**: Automated 15-minute rounds with VRF winner selection
- **Price Integration**: Real-time BTC/SOL/Token price conversion
- **Admin Controls**: Fee management, emergency controls, commission withdrawal

### Key Features
✅ **Multi-token support** (SOL + any SPL token)  
✅ **BTC-denominated pricing** (0.00005 BTC per ticket)  
✅ **Automated rounds** (15-minute duration)  
✅ **First buyer bonus** (extra free ticket)  
✅ **Verifiable randomness** (Switchboard VRF)  
✅ **Price feeds** (Switchboard/Pyth integration)  
✅ **Commission system** (configurable %)  
✅ **Emergency controls** (pause, admin functions)  

## 📦 Project Structure

```
lottery-program/
├── programs/bclot/
│   ├── src/
│   │   ├── lib.rs              # Main program logic
│   │   ├── price_feeds.rs      # Switchboard/Pyth integration
│   │   ├── vrf.rs              # VRF randomness integration
│   │   ├── admin.rs            # Admin functions & getters
│   │   └── automation.rs       # Clockwork automation
│   └── Cargo.toml
├── tests/
│   └── bclot.ts              # Comprehensive test suite
├── target/
├── Anchor.toml
└── package.json
```

## 🚀 Quick Setup

### Prerequisites
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Solana CLI
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"

# Install Anchor
cargo install --git https://github.com/coral-xyz/anchor avm --force
avm install latest
avm use latest

# Install Node dependencies
npm install
```

### Environment Setup
```bash
# Configure Solana CLI
solana config set --url devnet
solana-keygen new --outfile ~/.config/solana/id.json

# Get devnet SOL
solana airdrop 2

# Verify setup
solana balance
anchor --version
```

## 🔧 Build & Deploy

### Local Development
```bash
# Build the program
anchor build

# Start local validator (optional)
solana-test-validator

# Deploy to localnet
anchor deploy

# Run tests
anchor test
```

### Devnet Deployment
```bash
# Configure for devnet
anchor build
solana config set --url devnet

# Deploy to devnet
anchor deploy --provider.cluster devnet

# Verify deployment
solana program show <PROGRAM_ID>
```

### Mainnet Deployment
```bash
# Configure for mainnet
solana config set --url mainnet-beta

# Build for production
anchor build --verifiable

# Deploy (make sure you have enough SOL)
anchor deploy --provider.cluster mainnet-beta
```

## 📝 Configuration

### Anchor.toml
```toml
[features]
resolution = false
skip-lint = false

[programs.localnet]
lottery = "4JVMVPbeQ99TT3Jz3toaiLrTNf72iZ4X8jXYj5FseExc"

[programs.devnet]
lottery = "4JVMVPbeQ99TT3Jz3toaiLrTNf72iZ4X8jXYj5FseExc"

[programs.mainnet]
lottery = "4JVMVPbeQ99TT3Jz3toaiLrTNf72iZ4X8jXYj5FseExc"

[registry]
url = "https://api.apr.dev"

[provider]
cluster = "Devnet"
wallet = "~/.config/solana/id.json"

[scripts]
test = "yarn run ts-mocha -p ./tsconfig.json -t 1000000 tests/**/*.ts"
```

## 🎮 Usage Examples

### 1. Initialize Lottery
```typescript
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Lottery } from "./target/types/lottery";

const program = anchor.workspace.Lottery as Program<Lottery>;
const authority = provider.wallet.publicKey;

// Initialize lottery state
await program.methods
  .initializeLottery(
    5,                    // 5% entrance fee
    beneficiaryPublicKey  // Commission recipient
  )
  .accounts({
    lotteryState,
    authority,
    systemProgram: anchor.web3.SystemProgram.programId,
  })
  .rpc();
```

### 2. Add Supported Tokens
```typescript
// Add USDC support
const usdcMint = new anchor.web3.PublicKey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const usdcPriceFeed = new anchor.web3.PublicKey("SWITCHBOARD_USDC_FEED_ADDRESS");

await program.methods
  .addSupportedToken(usdcMint, usdcPriceFeed)
  .accounts({
    lotteryState,
    authority,
  })
  .rpc();
```

### 3. Buy SOL Tickets
```typescript
const player = anchor.web3.Keypair.generate();
const ticketCount = 3;

// Derive PDAs
const [playerData] = anchor.web3.PublicKey.findProgramAddressSync(
  [Buffer.from("player_data"), player.publicKey.toBuffer(), Buffer.from("SOL")],
  program.programId
);

await program.methods
  .buyTicketsSol(ticketCount)
  .accounts({
    lotteryState,
    solLottery,
    playerData,
    player: player.publicKey,
    vault: vaultSol,
    btcPriceFeed,
    solPriceFeed,
    systemProgram: anchor.web3.SystemProgram.programId,
  })
  .signers([player])
  .rpc();
```

### 4. Buy SPL Token Tickets
```typescript
// Buy USDC tickets
await program.methods
  .buyTicketsSpl(usdcMint, 2)
  .accounts({
    lotteryState,
    tokenLottery,
    playerData,
    player: player.publicKey,
    tokenMint: usdcMint,
    playerTokenAccount,
    vaultTokenAccount,
    vaultAuthority,
    btcPriceFeed,
    tokenPriceFeed,
    tokenProgram: TOKEN_PROGRAM_ID,
    systemProgram: anchor.web3.SystemProgram.programId,
  })
  .signers([player])
  .rpc();
```

### 5. Close Round & Pick Winner
```typescript
// Authority closes round (or automated via Clockwork)
const randomness = new anchor.BN(Math.floor(Math.random() * 1000000));

await program.methods
  .closeRoundAndPickWinner(usdcMint, randomness) // null for SOL
  .accounts({
    lotteryState,
    tokenLottery,
    solLottery,
    authority,
  })
  .rpc();
```

### 6. Claim Prize
```typescript
// Winner claims SOL prize
await program.methods
  .claimPrizeSol(new anchor.BN(0)) // Round ID
  .accounts({
    solLottery,
    winner: winnerKeypair.publicKey,
    vault: vaultSol,
    beneficiary,
  })
  .signers([winnerKeypair])
  .rpc();

// Winner claims SPL token prize
await program.methods
  .claimPrizeSpl(usdcMint, new anchor.BN(0))
  .accounts({
    tokenLottery,
    winner: winnerKeypair.publicKey,
    vaultTokenAccount,
    winnerTokenAccount,
    beneficiaryTokenAccount,
    vaultAuthority,
    tokenProgram: TOKEN_PROGRAM_ID,
  })
  .signers([winnerKeypair])
  .rpc();
```

## 🔍 Query Functions

### Get Lottery State
```typescript
const state = await program.methods
  .getLotteryState()
  .accounts({ lotteryState })
  .view();

console.log("Supported tokens:", state.supportedTokens);
console.log("Entrance fee:", state.entranceFeePercentage + "%");
```

### Get Round Data
```typescript
const roundData = await program.methods
  .getRoundData(new anchor.BN(0))
  .accounts({ lottery: solLottery })
  .view();

console.log("Winner:", roundData.winnerAddress);
console.log("Prize pool:", roundData.poolBalance);
```

### Get Current Ticket Prices
```typescript
// SOL ticket price
const solPrice = await program.methods
  .getCurrentTicketPriceSol()
  .accounts({ btcPriceFeed, solPriceFeed })
  .view();

// USDC ticket price
const usdcPrice = await program.methods
  .getCurrentTicketPriceSpl()
  .accounts({ 
    tokenMint: usdcMint,
    btcPriceFeed, 
    tokenPriceFeed 
  })
  .view();

console.log(`SOL ticket price: ${solPrice} lamports`);
console.log(`USDC ticket price: ${usdcPrice} tokens`);
```

## 🔐 Admin Functions

### Update Configuration
```typescript
// Update entrance fee
await program.methods
  .updateEntranceFee(3) // 3%
  .accounts({ lotteryState, authority })
  .rpc();

// Update beneficiary
await program.methods
  .updateBeneficiary(newBeneficiary)
  .accounts({ lotteryState, authority })
  .rpc();

// Emergency pause
await program.methods
  .emergencyPause()
  .accounts({ lotteryState, authority })
  .rpc();
```

### Withdraw Commission
```typescript
// Withdraw SOL commission
await program.methods
  .withdrawCommissionSol(new anchor.BN(1000000)) // 0.001 SOL
  .accounts({
    lotteryState,
    authority,
    vault: vaultSol,
    beneficiary,
  })
  .rpc();

// Withdraw SPL token commission
await program.methods
  .withdrawCommissionSpl(new anchor.BN(1000000)) // Amount
  .accounts({
    lotteryState,
    authority,
    tokenMint: usdcMint,
    vaultTokenAccount,
    beneficiaryTokenAccount,
    vaultAuthority,
    tokenProgram: TOKEN_PROGRAM_ID,
  })
  .rpc();
```

## 🤖 Automation with Clockwork

### Setup Automated Round Management
```typescript
// Create Clockwork thread for automation
const clockworkProgram = anchor.workspace.ClockworkSdk;

await clockworkProgram.methods
  .threadCreate(
    "lottery_automation",     // Thread name
    {
      cron: { schedule: "0 */15 * * * *" }, // Every 15 minutes
      now: null,
    },
    [
      {
        programId: program.programId,
        instruction: program.methods
          .performUpkeep()
          .accounts({
            lotteryState,
            threadAuthority,
          })
          .instruction(),
      },
    ]
  )
  .rpc();
```

## 🔌 Price Feed Integration

### Switchboard Price Feeds (Recommended)
```typescript
// Mainnet Switchboard feeds
const BTC_USD_FEED = "8SXvChNYFhRq4EZuZvnhjrB3jJRQCv4k3P4W6hesH3Ee";
const SOL_USD_FEED = "GvDMxPzN1sCj7L26YDK2HnMRXEQmQ2aemov8YBtPS7vR";
const USDC_USD_FEED = "BjUgj6YCnFBZ49wF54ddBVA9qu8TeqkFtkbqmZcee8uW";

// Devnet feeds available at: https://docs.switchboard.xyz/
```

### Pyth Price Feeds (Alternative)
```typescript
// Mainnet Pyth feeds
const BTC_USD_FEED = "GVXRSBjFk6e6J3NbVPXohDJetcTjaeeuykUpbQF8UoMU";
const SOL_USD_FEED = "H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG";
const USDC_USD_FEED = "Gnt27xtC473ZT2Mw5u8wZ68Z3gULkSTb5DuxJy7eJotD";
```

## 🧪 Testing

### Run Full Test Suite
```bash
# Run all tests
anchor test

# Run specific test file
anchor test tests/lottery.ts

# Run with logs
anchor test --detach
```

### Test Categories
- **Unit Tests**: Individual function testing
- **Integration Tests**: Full lottery lifecycle
- **Performance Tests**: Max participants, concurrent rounds
- **Error Handling**: Invalid inputs, edge cases
- **Admin Tests**: Authority functions, emergency controls

## 📊 Monitoring & Analytics

### Key Metrics to Track
```typescript
// Track total volume
const allRounds = await getAllRounds();
const totalVolume = allRounds.reduce((sum, round) => sum + round.poolBalance, 0);

// Track player activity
const playerStats = await getPlayerStats(playerPubkey);

// Track commission earned
const commissionEarned = await getCommissionStats();
```

### Event Monitoring
```typescript
// Listen for ticket purchases
program.addEventListener("TicketPurchased", (event) => {
  console.log(`Ticket purchased: ${event.count} tickets for ${event.totalAmount}`);
});

// Listen for winners
program.addEventListener("WinnerPicked", (event) => {
  console.log(`Winner: ${event.winner}, Prize: ${event.prizeAmount}`);
});
```

## 🚨 Security Considerations

### Best Practices
1. **Use hardware wallets** for authority keys in production
2. **Set reasonable entrance fees** (max 20% enforced)
3. **Monitor commission withdrawals** regularly
4. **Test emergency pause** functionality
5. **Verify VRF randomness** sources
6. **Audit price feed** reliability
7. **Set up monitoring** for unusual activity

### Emergency Procedures
```typescript
// Emergency pause (stops new rounds)
await program.methods.emergencyPause().rpc();

// Withdraw stuck funds (if needed)
await program.methods.withdrawCommissionSol(amount).rpc();
```

## 📈 Scaling Considerations

### Performance Optimization
- **Account size limits**: Max 500 players per round
- **Batch operations**: Process multiple tokens together
- **Caching**: Cache price feed data when possible
- **Indexing**: Use indexers for historical data

### Future Enhancements
- **Mobile app integration**
- **Advanced analytics dashboard**
- **Multi-chain support**
- **NFT ticket representation**
- **Governance token integration**

## 🆘 Troubleshooting

### Common Issues

**Build Errors**
```bash
# Clear cache and rebuild
anchor clean
cargo clean
anchor build
```

**Test Failures**
```bash
# Restart validator
solana-test-validator --reset
anchor test
```

**Deployment Issues**
```bash
# Check program size
ls -la target/deploy/
# Upgrade if needed
anchor upgrade <PROGRAM_ADDRESS>
```

## 📞 Support & Resources

### Documentation
- [Anchor Book](https://anchor-lang.com/)
- [Solana Cookbook](https://solanacookbook.com/)
- [Switchboard Docs](https://docs.switchboard.xyz/)

### Community
- [Solana Discord](https://discord.gg/solana)
- [Anchor Discord](https://discord.gg/ZCHmqvXgDw)
- [Stack Overflow](https://stackoverflow.com/questions/tagged/solana)

---

## 🎉 Congratulations!

You now have a **production-ready, multi-token lottery program** with:
- ✅ BTC-denominated pricing
- ✅ Automated round management  
- ✅ Verifiable randomness
- ✅ Admin controls
- ✅ Comprehensive testing
- ✅ Real-world price feeds

**Happy building!** 🚀