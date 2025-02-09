import { TOKEN_PROGRAM_ID } from '@solana/spl-token';
import BN from 'bn.js';

describe("flash-liquidity-token", () => {
  it("updates governance parameters", async () => {
    // Generate keypair for the governance account.
    const governanceKp = new web3.Keypair();

    // Call updateGovernanceParameters with sample values.
    const txHash = await pg.program.methods
      .updateGovernanceParameters(
        new BN(20),   // flash_loan_fee_bps (default fee, unused in dynamic mode)
        new BN(50),   // liquidation_penalty_bps (e.g., 0.50%)
        new BN(10),   // liquidation_grace_slots (in slots)
        new BN(1),    // compound_rate_numerator
        new BN(100),  // compound_rate_denominator
        new BN(5000)  // max_borrow_ratio (50.00%)
      )
      .accounts({
        admin: pg.wallet.publicKey,
        governance: governanceKp.publicKey,
      })
      .signers([governanceKp])
      .rpc();
    console.log("Governance updated, txHash:", txHash);
    await pg.connection.confirmTransaction(txHash);
  });

  it("stakes collateral", async () => {
    // Generate keypairs for collateral mint and FLT mint.
    const collateralMintKp = new web3.Keypair();
    const fltMintKp = new web3.Keypair();
    const fltMintWrapper = { bump: 255 }; // Dummy bump value for the FLT mint PDA.

    // Generate token accounts for the user and the vault.
    const userTokenAccountKp = new web3.Keypair();
    const vaultTokenAccountKp = new web3.Keypair();
    const userFlTokenAccountKp = new web3.Keypair();

    // Derive the staker PDA using seeds: ["staker", user public key, collateral mint]
    const [stakerPda, _stakerBump] = await web3.PublicKey.findProgramAddress(
      [
        Buffer.from("staker"),
        pg.wallet.publicKey.toBuffer(),
        collateralMintKp.publicKey.toBuffer(),
      ],
      pg.program.programId
    );

    // Derive the vault PDA using seeds: ["vault", collateral mint]
    const [vaultPda, _vaultBump] = await web3.PublicKey.findProgramAddress(
      [Buffer.from("vault"), collateralMintKp.publicKey.toBuffer()],
      pg.program.programId
    );

    // Generate new keypairs for governance and reward pool accounts.
    const governanceKp = new web3.Keypair();
    const rewardPoolKp = new web3.Keypair();

    // Define the collateral amount and lock duration.
    const stakeAmount = new BN(1000000);
    const lockDuration = new BN(100);

    // Call the stake instruction.
    const txHash = await pg.program.methods
      .stake(stakeAmount, lockDuration)
      .accounts({
        user: pg.wallet.publicKey,
        userTokenAccount: userTokenAccountKp.publicKey,
        vaultTokenAccount: vaultTokenAccountKp.publicKey,
        collateralMint: collateralMintKp.publicKey,
        fltMint: fltMintKp.publicKey,
        fltMintWrapper: fltMintWrapper,
        userFltTokenAccount: userFlTokenAccountKp.publicKey,
        governance: governanceKp.publicKey,
        rewardPool: rewardPoolKp.publicKey,
        staker: stakerPda,
        vaultAccount: vaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: web3.SystemProgram.programId,
        rent: web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();
    console.log("Stake txHash:", txHash);
    await pg.connection.confirmTransaction(txHash);
  });

  it("borrows liquidity", async () => {
    // Generate keypairs for collateral mint and FLT mint.
    const collateralMintKp = new web3.Keypair();
    const fltMintKp = new web3.Keypair();
    const fltMintWrapper = { bump: 255 };

    // Generate token accounts.
    const userTokenAccountKp = new web3.Keypair();
    const vaultTokenAccountKp = new web3.Keypair();
    const userFlTokenAccountKp = new web3.Keypair();

    // Derive PDAs for staker and vault.
    const [stakerPda, _stakerBump] = await web3.PublicKey.findProgramAddress(
      [
        Buffer.from("staker"),
        pg.wallet.publicKey.toBuffer(),
        collateralMintKp.publicKey.toBuffer(),
      ],
      pg.program.programId
    );
    const [vaultPda, _vaultBump] = await web3.PublicKey.findProgramAddress(
      [Buffer.from("vault"), collateralMintKp.publicKey.toBuffer()],
      pg.program.programId
    );

    // Generate new keypairs for governance and reward pool accounts.
    const governanceKp = new web3.Keypair();
    const rewardPoolKp = new web3.Keypair();

    // Create dummy accounts for the callback program and Pyth price.
    const callbackProgramKp = new web3.Keypair();
    const pythPriceKp = new web3.Keypair();

    // Generate a new loan account.
    const loanKp = new web3.Keypair();

    // Define borrow amount and loan duration.
    const borrowAmount = new BN(500000);
    const loanDuration = new BN(50);

    // Call the borrow instruction.
    const txHash = await pg.program.methods
      .borrow(borrowAmount, loanDuration)
      .accounts({
        borrower: pg.wallet.publicKey,
        borrowerTokenAccount: userTokenAccountKp.publicKey,
        vaultAccount: vaultPda,
        vaultTokenAccount: vaultTokenAccountKp.publicKey,
        staker: stakerPda,
        loan: loanKp.publicKey,
        governance: governanceKp.publicKey,
        rewardPool: rewardPoolKp.publicKey,
        callbackProgram: callbackProgramKp.publicKey,
        pythPrice: pythPriceKp.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: web3.SystemProgram.programId,
        rent: web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([loanKp])
      .rpc();
    console.log("Borrow txHash:", txHash);
    await pg.connection.confirmTransaction(txHash);
  });

  // Additional tests (such as for repay, compoundRewards, unstake, liquidate) can be added below.
  it("repays loan", async () => {
    // Insert test logic for repay here.
    console.log("Repay test not implemented yet.");
  });

  it("compounds rewards", async () => {
    // Insert test logic for compoundRewards here.
    console.log("Compound rewards test not implemented yet.");
  });

  it("unstakes collateral", async () => {
    // Insert test logic for unstake here.
    console.log("Unstake test not implemented yet.");
  });

  it("liquidates overdue loan", async () => {
    // Insert test logic for liquidate here.
    console.log("Liquidation test not implemented yet.");
  });
});
