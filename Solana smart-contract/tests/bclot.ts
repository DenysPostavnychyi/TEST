// tests/lottery.ts - Comprehensive test suite
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Lottery } from "../target/types/lottery";
import { assert } from "chai";
import { 
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount
} from "@solana/spl-token";

describe("Lottery Program Tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Lottery as Program<Lottery>;
  const authority = provider.wallet.publicKey;

  let lotteryState: anchor.web3.PublicKey;
  let solLottery: anchor.web3.PublicKey;
  let tokenMint: anchor.web3.PublicKey;
  let tokenLottery: anchor.web3.PublicKey;
  let vaultSol: anchor.web3.PublicKey;
  let vaultToken: anchor.web3.PublicKey;
  let vaultAuthority: anchor.web3.PublicKey;

  const beneficiary = anchor.web3.Keypair.generate().publicKey;
  const entranceFeePercentage = 5; // 5%

  before(async () => {
    // Derive PDAs
    [lotteryState] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("lottery_state")],
      program.programId
    );

    [solLottery] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("sol_lottery")],
      program.programId
    );

    [vaultSol] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("sol_vault")],
      program.programId
    );

    // Create test token
    tokenMint = await createMint(
      provider.connection,
      provider.wallet.payer,
      authority,
      authority,
      6 // USDC decimals
    );

    [tokenLottery] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("token_lottery"), tokenMint.toBuffer()],
      program.programId
    );

    [vaultToken] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), tokenMint.toBuffer()],
      program.programId
    );

    [vaultAuthority] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault_authority"), tokenMint.toBuffer()],
      program.programId
    );
  });

  describe("Initialization", () => {
    it("Should initialize lottery state", async () => {
      await program.methods
        .initializeLottery(entranceFeePercentage, beneficiary)
        .accounts({
          lotteryState,
          authority,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      const state = await program.account.lotteryState.fetch(lotteryState);
      assert.equal(state.authority.toString(), authority.toString());
      assert.equal(state.entranceFeePercentage, entranceFeePercentage);
      assert.equal(state.beneficiary.toString(), beneficiary.toString());
      assert.equal(state.hasActiveLottery, false);
    });

    it("Should add supported token", async () => {
      const mockPriceFeed = anchor.web3.Keypair.generate().publicKey;

      await program.methods
        .addSupportedToken(tokenMint, mockPriceFeed)
        .accounts({
          lotteryState,
          authority,
        })
        .rpc();

      const state = await program.account.lotteryState.fetch(lotteryState);
      assert.equal(state.supportedTokens.length, 1);
      assert.equal(state.supportedTokens[0].mint.toString(), tokenMint.toString());
    });
  });

  describe("SOL Lottery", () => {
    const player = anchor.web3.Keypair.generate();
    let playerData: anchor.web3.PublicKey;

    before(async () => {
      // Airdrop SOL to player
      await provider.connection.requestAirdrop(player.publicKey, 2 * anchor.web3.LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));

      [playerData] = anchor.web3.PublicKey.findProgramAddressSync(
        [Buffer.from("player_data"), player.publicKey.toBuffer(), Buffer.from("SOL")],
        program.programId
      );
    });

    it("Should buy SOL tickets", async () => {
      const ticketCount = 3;
      const mockBtcPriceFeed = anchor.web3.Keypair.generate().publicKey;
      const mockSolPriceFeed = anchor.web3.Keypair.generate().publicKey;

      await program.methods
        .buyTicketsSol(ticketCount)
        .accounts({
          lotteryState,
          solLottery,
          playerData,
          player: player.publicKey,
          vault: vaultSol,
          btcPriceFeed: mockBtcPriceFeed,
          solPriceFeed: mockSolPriceFeed,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([player])
        .rpc();

      const lottery = await program.account.tokenLottery.fetch(solLottery);
      assert.equal(lottery.rounds.length, 1);
      
      const round = lottery.rounds[0];
      assert.equal(round.tickets.length, ticketCount + 1); // +1 for bonus ticket
      assert.equal(round.status.open !== undefined, true);
    });

    it("Should close round and pick winner", async () => {
      const randomness = new anchor.BN(12345);

      await program.methods
        .closeRoundAndPickWinner(null, randomness)
        .accounts({
          lotteryState,
          tokenLottery: solLottery,
          solLottery,
          authority,
        })
        .rpc();

      const lottery = await program.account.tokenLottery.fetch(solLottery);
      const round = lottery.rounds[0];
      
      assert.equal(round.status.closed !== undefined, true);
      assert.isNotNull(round.winnerAddress);
      assert.isNotNull(round.winnerTicketIndex);
    });

    it("Should claim SOL prize", async () => {
      const roundId = new anchor.BN(0);
      const initialBalance = await provider.connection.getBalance(player.publicKey);

      await program.methods
        .claimPrizeSol(roundId)
        .accounts({
          solLottery,
          winner: player.publicKey,
          vault: vaultSol,
          beneficiary,
        })
        .signers([player])
        .rpc();

      const finalBalance = await provider.connection.getBalance(player.publicKey);
      assert.isTrue(finalBalance > initialBalance);

      const lottery = await program.account.tokenLottery.fetch(solLottery);
      const round = lottery.rounds[0];
      assert.equal(round.prizeClaimed, true);
    });
  });

  describe("SPL Token Lottery", () => {
    const player = anchor.web3.Keypair.generate();
    let playerTokenAccount: anchor.web3.PublicKey;
    let playerData: anchor.web3.PublicKey;

    before(async () => {
      // Airdrop SOL for transaction fees
      await provider.connection.requestAirdrop(player.publicKey, anchor.web3.LAMPORTS_PER_SOL);
      await new Promise(resolve => setTimeout(resolve, 1000));

      // Create token account for player
      playerTokenAccount = await createAccount(
        provider.connection,
        provider.wallet.payer,
        tokenMint,
        player.publicKey
      );

      // Mint tokens to player
      await mintTo(
        provider.connection,
        provider.wallet.payer,
        tokenMint,
        playerTokenAccount,
        authority,
        1000000000 // 1000 USDC (6 decimals)
      );

      [playerData] = anchor.web3.PublicKey.findProgramAddressSync(
        [Buffer.from("player_data"), player.publicKey.toBuffer(), tokenMint.toBuffer()],
        program.programId
      );
    });

    it("Should buy SPL token tickets", async () => {
      const ticketCount = 2;
      const mockBtcPriceFeed = anchor.web3.Keypair.generate().publicKey;
      const mockTokenPriceFeed = anchor.web3.Keypair.generate().publicKey;

      await program.methods
        .buyTicketsSpl(tokenMint, ticketCount)
        .accounts({
          lotteryState,
          tokenLottery,
          playerData,
          player: player.publicKey,
          tokenMint,
          playerTokenAccount,
          vaultTokenAccount: vaultToken,
          vaultAuthority,
          btcPriceFeed: mockBtcPriceFeed,
          tokenPriceFeed: mockTokenPriceFeed,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([player])
        .rpc();

      const lottery = await program.account.tokenLottery.fetch(tokenLottery);
      assert.equal(lottery.rounds.length, 1);
      
      const round = lottery.rounds[0];
      assert.equal(round.tickets.length, ticketCount + 1); // +1 for bonus ticket
    });

    it("Should close token round and pick winner", async () => {
      const randomness = new anchor.BN(54321);

      await program.methods
        .closeRoundAndPickWinner(tokenMint, randomness)
        .accounts({
          lotteryState,
          tokenLottery,
          solLottery,
          authority,
        })
        .rpc();

      const lottery = await program.account.tokenLottery.fetch(tokenLottery);
      const round = lottery.rounds[0];
      
      assert.equal(round.status.closed !== undefined, true);
      assert.isNotNull(round.winnerAddress);
    });
  });

  describe("Admin Functions", () => {
    it("Should update entrance fee", async () => {
      const newFee = 3;

      await program.methods
        .updateEntranceFee(newFee)
        .accounts({
          lotteryState,
          authority,
        })
        .rpc();

      const state = await program.account.lotteryState.fetch(lotteryState);
      assert.equal(state.entranceFeePercentage, newFee);
    });

    it("Should update beneficiary", async () => {
      const newBeneficiary = anchor.web3.Keypair.generate().publicKey;

      await program.methods
        .updateBeneficiary(newBeneficiary)
        .accounts({
          lotteryState,
          authority,
        })
        .rpc();

      const state = await program.account.lotteryState.fetch(lotteryState);
      assert.equal(state.beneficiary.toString(), newBeneficiary.toString());
    });

    it("Should emergency pause", async () => {
      await program.methods
        .emergencyPause()
        .accounts({
          lotteryState,
          authority,
        })
        .rpc();

      const state = await program.account.lotteryState.fetch(lotteryState);
      assert.equal(state.hasActiveLottery, false);
    });
  });

  describe("View Functions", () => {
    it("Should get lottery state", async () => {
      const result = await program.methods
        .getLotteryState()
        .accounts({
          lotteryState,
        })
        .view();

      assert.equal(result.authority.toString(), authority.toString());
      assert.isArray(result.supportedTokens);
    });

    it("Should get round data", async () => {
      const roundId = new anchor.BN(0);

      const result = await program.methods
        .getRoundData(roundId)
        .accounts({
          lottery: solLottery,
        })
        .view();

      assert.equal(result.roundId.toString(), roundId.toString());
      assert.isTrue(result.ticketCount > 0);
    });

    it("Should get current ticket price for SOL", async () => {
      const mockBtcPriceFeed = anchor.web3.Keypair.generate().publicKey;
      const mockSolPriceFeed = anchor.web3.Keypair.generate().publicKey;

      const price = await program.methods
        .getCurrentTicketPriceSol()
        .accounts({
          btcPriceFeed: mockBtcPriceFeed,
          solPriceFeed: mockSolPriceFeed,
        })
        .view();

      assert.isTrue(price.gt(new anchor.BN(0)));
    });
  });

  describe("Automation", () => {
    it("Should check upkeep", async () => {
      const upkeepNeeded = await program.methods
        .checkUpkeep()
        .accounts({
          lotteryState,
        })
        .view();

      assert.isBoolean(upkeepNeeded);
    });
  });

  describe("Error Handling", () => {
    it("Should fail with invalid entrance fee", async () => {
      try {
        await program.methods
          .initializeLottery(25, beneficiary) // 25% is > 20% limit
          .accounts({
            lotteryState: anchor.web3.Keypair.generate().publicKey,
            authority,
            systemProgram: anchor.web3.SystemProgram.programId,
          })
          .rpc();
        assert.fail("Should have thrown error");
      } catch (error) {
        assert.include(error.toString(), "InvalidEntranceFee");
      }
    });

    it("Should fail to buy too many tickets", async () => {
      const player = anchor.web3.Keypair.generate();
      
      try {
        await program.methods
          .buyTicketsSol(6) // Max is 5
          .accounts({
            lotteryState,
            solLottery,
            playerData: anchor.web3.Keypair.generate().publicKey,
            player: player.publicKey,
            vault: vaultSol,
            btcPriceFeed: anchor.web3.Keypair.generate().publicKey,
            solPriceFeed: anchor.web3.Keypair.generate().publicKey,
            systemProgram: anchor.web3.SystemProgram.programId,
          })
          .signers([player])
          .rpc();
        assert.fail("Should have thrown error");
      } catch (error) {
        assert.include(error.toString(), "InvalidTicketCount");
      }
    });
  });
});

// Integration test with real price feeds (for devnet/mainnet)
describe("Integration Tests", () => {
  // These tests would use real Switchboard price feeds
  // and test the full lottery lifecycle
  
  it("Should integrate with real price feeds", async () => {
    // Implementation for testing with real Switchboard feeds
    console.log("Integration tests would go here");
  });

  it("Should integrate with VRF", async () => {
    // Implementation for testing with real Switchboard VRF
    console.log("VRF integration tests would go here");
  });
});

// Performance tests
describe("Performance Tests", () => {
  it("Should handle maximum participants", async () => {
    // Test with maximum number of participants (500)
    console.log("Performance tests would go here");
  });

  it("Should handle multiple concurrent rounds", async () => {
    // Test multiple tokens with concurrent rounds
    console.log("Concurrent round tests would go here");
  });
});