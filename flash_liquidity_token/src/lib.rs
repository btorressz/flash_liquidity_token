use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer, MintTo};
use anchor_lang::solana_program::native_token::LAMPORTS_PER_SOL;
use pyth_sdk_solana::load_price_feed_from_account_info;

declare_id!("BVN4FsG6E67eboE2nK6yHZkh7segTJ2KfQfiZhjPoQDk");

#[program]
pub mod flash_liquidity_token {
    use super::*;

    /// Stake collateral to mint FLT tokens.
    /// The staker specifies the amount and lock duration (in slots).
    /// Early stakers (when total staked < 10,000 SOL) receive a 1.5x boost.
    pub fn stake(ctx: Context<Stake>, amount: u64, lock_duration: u64) -> Result<()> {
        let clock = Clock::get()?;
        let current_slot = clock.slot;

        // Validate collateral mint.
        require!(
            ctx.accounts.user_token_account.mint == ctx.accounts.collateral_mint.key(),
            CustomError::InvalidCollateralMint
        );
        require!(
            ctx.accounts.governance.supported_collaterals.contains(&ctx.accounts.collateral_mint.key()),
            CustomError::UnsupportedCollateral
        );

        // Transfer collateral from the user to the vault.
        let transfer_cpi_accounts = Transfer {
            from: ctx.accounts.user_token_account.to_account_info(),
            to: ctx.accounts.vault_token_account.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), transfer_cpi_accounts),
            amount,
        )?;

        // Mint FLT tokens to the user.
        let seeds = &[ctx.accounts.flt_mint.to_account_info().key.as_ref(), &[ctx.accounts.flt_mint_wrapper.bump]];
        let signer = &[&seeds[..]];
        let mint_to_cpi_accounts = MintTo {
            mint: ctx.accounts.flt_mint.to_account_info(),
            to: ctx.accounts.user_flt_token_account.to_account_info(),
            authority: ctx.accounts.flt_mint.to_account_info(),
        };
        token::mint_to(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), mint_to_cpi_accounts, signer),
            amount,
        )?;

        // Reward Boosting for Early Adopters:
        // If the total staked is below 10,000 SOL, apply a 1.5x multiplier.
        let boosted_amount = if ctx.accounts.reward_pool.total_staked < 10_000 * LAMPORTS_PER_SOL {
            amount.checked_mul(150).unwrap().checked_div(100).unwrap()
        } else {
            amount
        };

        // Update or initialize the staker record.
        let staker = &mut ctx.accounts.staker;
        staker.staked_amount = staker.staked_amount.checked_add(boosted_amount).unwrap();
        staker.collateral_mint = ctx.accounts.collateral_mint.key();
        staker.last_compound_slot = current_slot;
        staker.lock_end_slot = current_slot.checked_add(lock_duration).unwrap();

        // Update the global reward pool.
        ctx.accounts.reward_pool.total_staked = ctx.accounts.reward_pool.total_staked.checked_add(boosted_amount).unwrap();
        ctx.accounts.reward_pool.update_counter = ctx.accounts.reward_pool.update_counter.checked_add(1).unwrap();

        Ok(())
    }

    /// Borrow liquidity for a short duration.
    /// The dynamic flash loan fee is computed based on utilization and adjusted via the Pyth oracle.
    /// After transferring funds, the program calls a callback program for atomic arbitrage.
    pub fn borrow(ctx: Context<Borrow>, amount: u64, loan_duration: u64) -> Result<()> {
        let clock = Clock::get()?;
        let current_slot = clock.slot;
        let current_time_i64 = clock.unix_timestamp;

        // Ensure the timestamp is non-negative before conversion.
        require!(current_time_i64 >= 0, CustomError::InvalidTimestamp);
        let current_time: u64 = current_time_i64 as u64;

        // Reentrancy protection.
        require!(!ctx.accounts.loan.active, CustomError::ReentrancyDetected);

        // Compute utilization.
        let new_utilization = ctx.accounts
            .reward_pool
            .active_loan_total
            .checked_add(amount)
            .unwrap()
            .checked_mul(100)
            .unwrap()
            .checked_div(ctx.accounts.reward_pool.total_staked)
            .unwrap();
        let mut flash_fee_bps: u64 = if new_utilization < 20 {
            15  // 0.15%
        } else if new_utilization < 80 {
            20  // 0.20%
        } else {
            50  // 0.50%
        };

        // Oracle Integration: Read Pyth price to adjust the fee.
        let price_feed = load_price_feed_from_account_info(&ctx.accounts.pyth_price)
            .map_err(|_| ProgramError::Custom(CustomError::OraclePriceUnavailable as u32))?;
        // Use get_price_no_older_than with a 60-second threshold and the current timestamp.
        let price_info = price_feed
            .get_price_no_older_than(60, current_time)
            .ok_or(ProgramError::Custom(CustomError::OraclePriceUnavailable as u32))?;
        if price_info.price > 0 {
            flash_fee_bps = flash_fee_bps
                .checked_mul(100)
                .unwrap()
                .checked_div(price_info.price as u64)
                .unwrap();
        }

        let flash_fee = amount.checked_mul(flash_fee_bps).unwrap().checked_div(10000).unwrap();
        let amount_after_fee = amount.checked_sub(flash_fee).unwrap();

        // Enforce collateralized borrowing: loan amount must be within allowed ratio.
        let staker = &ctx.accounts.staker;
        require!(
            amount <= staker
                .staked_amount
                .checked_mul(ctx.accounts.governance.max_borrow_ratio)
                .unwrap()
                .checked_div(10000)
                .unwrap(),
            CustomError::BorrowAmountExceedsCollateral
        );

        // Record loan details and mark active.
        let loan = &mut ctx.accounts.loan;
        loan.borrower = ctx.accounts.borrower.key();
        loan.amount = amount;
        loan.start_slot = current_slot;
        loan.due_slot = current_slot.checked_add(loan_duration).unwrap();
        loan.active = true;

        // Update active loan total in reward pool.
        ctx.accounts.reward_pool.active_loan_total = ctx.accounts.reward_pool.active_loan_total.checked_add(amount).unwrap();
        ctx.accounts.reward_pool.update_counter = ctx.accounts.reward_pool.update_counter.checked_add(1).unwrap();

        // Transfer liquidity from the vault to the borrower.
        let seeds = &[b"vault", staker.collateral_mint.as_ref(), &[ctx.accounts.vault_account.bump]];
        let signer = &[&seeds[..]];
        let transfer_cpi_accounts = Transfer {
            from: ctx.accounts.vault_token_account.to_account_info(),
            to: ctx.accounts.borrower_token_account.to_account_info(),
            authority: ctx.accounts.vault_account.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), transfer_cpi_accounts, signer),
            amount_after_fee,
        )?;

        // Credit the flash fee to the reward pool.
        ctx.accounts.reward_pool.accrued_fees = ctx.accounts.reward_pool.accrued_fees.checked_add(flash_fee).unwrap();

        // Flash Loan Callback:
        // After transferring liquidity, invoke the callback program.
        let callback_ix = anchor_lang::solana_program::instruction::Instruction {
            program_id: ctx.accounts.callback_program.key(),
            accounts: vec![
                AccountMeta::new(ctx.accounts.borrower.key(), true),
                AccountMeta::new(ctx.accounts.borrower_token_account.key(), false),
            ],
            data: vec![], // Insert callback-specific data here.
        };
        anchor_lang::solana_program::program::invoke(
            &callback_ix,
            &[
                ctx.accounts.callback_program.to_account_info(),
                ctx.accounts.borrower.to_account_info(),
                ctx.accounts.borrower_token_account.to_account_info(),
            ],
        )?;

        Ok(())
    }

    /// Repay the borrowed liquidity.
    /// If repaid late, a penalty fee is applied.
    pub fn repay(ctx: Context<Repay>, amount: u64) -> Result<()> {
        let clock = Clock::get()?;
        let current_slot = clock.slot;
        let loan = &mut ctx.accounts.loan;

        require!(loan.active, CustomError::LoanNotActive);

        let mut penalty_fee: u64 = 0;
        if current_slot > loan.due_slot {
            let overdue_slots = current_slot.checked_sub(loan.due_slot).unwrap();
            penalty_fee = loan.amount
                .checked_mul(overdue_slots)
                .unwrap()
                .checked_mul(ctx.accounts.governance.liquidation_penalty_bps)
                .unwrap()
                .checked_div(10000)
                .unwrap();
        }
        let total_required = loan.amount.checked_add(penalty_fee).unwrap();
        require!(amount >= total_required, CustomError::RepaymentFeeMissing);

        let transfer_cpi_accounts = Transfer {
            from: ctx.accounts.borrower_token_account.to_account_info(),
            to: ctx.accounts.vault_token_account.to_account_info(),
            authority: ctx.accounts.borrower.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), transfer_cpi_accounts),
            amount,
        )?;

        ctx.accounts.reward_pool.active_loan_total = ctx.accounts.reward_pool.active_loan_total.checked_sub(loan.amount).unwrap();
        if penalty_fee > 0 {
            ctx.accounts.reward_pool.accrued_fees = ctx.accounts.reward_pool.accrued_fees.checked_add(penalty_fee).unwrap();
        }
        loan.active = false;
        ctx.accounts.reward_pool.update_counter = ctx.accounts.reward_pool.update_counter.checked_add(1).unwrap();

        Ok(())
    }

    /// Liquidate an overdue loan.
    /// If a loan is past its due slot plus a grace period, a liquidator can seize collateral.
    pub fn liquidate(ctx: Context<Liquidate>) -> Result<()> {
        let clock = Clock::get()?;
        let current_slot = clock.slot;
        let loan = &mut ctx.accounts.loan;

        // Ensure the loan is overdue (including grace period).
        require!(
            current_slot > loan.due_slot.checked_add(ctx.accounts.governance.liquidation_grace_slots).unwrap(),
            CustomError::LoanNotOverdue
        );

        // Calculate penalty collateral (as an incentive to liquidators).
        let penalty_collateral = loan.amount
            .checked_mul(ctx.accounts.governance.liquidation_penalty_bps)
            .unwrap()
            .checked_div(10000)
            .unwrap();

        // Transfer penalty collateral from the vault to the liquidator.
        let seeds = &[b"vault", ctx.accounts.staker.collateral_mint.as_ref(), &[ctx.accounts.vault_account.bump]];
        let signer = &[&seeds[..]];
        let transfer_cpi_accounts = Transfer {
            from: ctx.accounts.vault_token_account.to_account_info(),
            to: ctx.accounts.liquidator_token_account.to_account_info(),
            authority: ctx.accounts.vault_account.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), transfer_cpi_accounts, signer),
            penalty_collateral,
        )?;

        // Mark the loan as inactive and update global state.
        loan.active = false;
        ctx.accounts.reward_pool.active_loan_total = ctx.accounts.reward_pool.active_loan_total.checked_sub(loan.amount).unwrap();
        ctx.accounts.reward_pool.accrued_fees = ctx.accounts.reward_pool.accrued_fees.checked_add(penalty_collateral).unwrap();
        ctx.accounts.reward_pool.update_counter = ctx.accounts.reward_pool.update_counter.checked_add(1).unwrap();

        Ok(())
    }

    /// Compound rewards for a staker.
    /// Additional rewards are calculated based on slots elapsed since the last compounding.
    pub fn compound_rewards(ctx: Context<CompoundRewards>) -> Result<()> {
        let clock = Clock::get()?;
        let current_slot = clock.slot;
        let staker = &mut ctx.accounts.staker;
        let slots_passed = current_slot.checked_sub(staker.last_compound_slot).unwrap();
        let rate_numerator = ctx.accounts.governance.compound_rate_numerator;
        let rate_denominator = ctx.accounts.governance.compound_rate_denominator;
        let additional_rewards = staker
            .staked_amount
            .checked_mul(rate_numerator)
            .unwrap()
            .checked_mul(slots_passed)
            .unwrap()
            .checked_div(rate_denominator)
            .unwrap();
        staker.staked_amount = staker.staked_amount.checked_add(additional_rewards).unwrap();
        staker.last_compound_slot = current_slot;
        ctx.accounts.reward_pool.total_staked = ctx.accounts.reward_pool.total_staked.checked_add(additional_rewards).unwrap();
        ctx.accounts.reward_pool.update_counter = ctx.accounts.reward_pool.update_counter.checked_add(1).unwrap();
        Ok(())
    }

    /// Unstake collateral after the lock period has expired.
    pub fn unstake(ctx: Context<Unstake>, amount: u64) -> Result<()> {
        let clock = Clock::get()?;
        let current_slot = clock.slot;
        let staker = &mut ctx.accounts.staker;
        require!(current_slot >= staker.lock_end_slot, CustomError::StakingLocked);
        require!(staker.staked_amount >= amount, CustomError::InsufficientStakedAmount);

        staker.staked_amount = staker.staked_amount.checked_sub(amount).unwrap();
        ctx.accounts.reward_pool.total_staked = ctx.accounts.reward_pool.total_staked.checked_sub(amount).unwrap();
        ctx.accounts.reward_pool.update_counter = ctx.accounts.reward_pool.update_counter.checked_add(1).unwrap();

        let seeds = &[b"vault", staker.collateral_mint.as_ref(), &[ctx.accounts.vault_account.bump]];
        let signer = &[&seeds[..]];
        let transfer_cpi_accounts = Transfer {
            from: ctx.accounts.vault_token_account.to_account_info(),
            to: ctx.accounts.user_token_account.to_account_info(),
            authority: ctx.accounts.vault_account.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), transfer_cpi_accounts, signer),
            amount,
        )?;

        Ok(())
    }

    /// Update governance parameters.
    pub fn update_governance_parameters(
        ctx: Context<UpdateGovernanceParameters>,
        flash_loan_fee_bps: u64,         // default fee (unused in dynamic mode)
        liquidation_penalty_bps: u64,
        liquidation_grace_slots: u64,
        compound_rate_numerator: u64,
        compound_rate_denominator: u64,
        max_borrow_ratio: u64,
    ) -> Result<()> {
        let governance = &mut ctx.accounts.governance;
        governance.flash_loan_fee_bps = flash_loan_fee_bps;
        governance.liquidation_penalty_bps = liquidation_penalty_bps;
        governance.liquidation_grace_slots = liquidation_grace_slots;
        governance.compound_rate_numerator = compound_rate_numerator;
        governance.compound_rate_denominator = compound_rate_denominator;
        governance.max_borrow_ratio = max_borrow_ratio;
        Ok(())
    }
}

//
// Account Contexts
//

#[derive(Accounts)]
pub struct Stake<'info> {
    /// The user staking tokens.
    #[account(mut)]
    pub user: Signer<'info>,
    /// The user's token account holding collateral.
    #[account(mut)]
    pub user_token_account: Box<Account<'info, TokenAccount>>,
    /// The vault token account (PDA) where collateral is stored.
    #[account(mut)]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,
    /// The collateral mint.
    pub collateral_mint: Box<Account<'info, Mint>>,
    /// The FLT mint corresponding to this collateral.
    #[account(mut)]
    pub flt_mint: Box<Account<'info, Mint>>,
    /// Helper account storing the bump for the FLT mint PDA.
    pub flt_mint_wrapper: Account<'info, MintWrapper>,
    /// The user's token account to receive minted FLT tokens.
    #[account(mut)]
    pub user_flt_token_account: Box<Account<'info, TokenAccount>>,
    /// The governance account.
    pub governance: Box<Account<'info, Governance>>,
    /// Global reward pool account.
    #[account(mut)]
    pub reward_pool: Box<Account<'info, RewardPool>>,
    /// Staker record (tracked per user per collateral type).
    #[account(
        init_if_needed,
        payer = user,
        space = Staker::LEN,
        seeds = [b"staker", user.key().as_ref(), collateral_mint.key().as_ref()],
        bump
    )]
    pub staker: Box<Account<'info, Staker>>,
    /// The vault PDA account, derived as: seeds = [b"vault", collateral_mint.key().as_ref()].
    #[account(
        mut,
        seeds = [b"vault", collateral_mint.key().as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, VaultAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Borrow<'info> {
    /// The borrower.
    #[account(mut)]
    pub borrower: Signer<'info>,
    /// The borrower's token account to receive liquidity.
    #[account(mut)]
    pub borrower_token_account: Box<Account<'info, TokenAccount>>,
    /// The vault PDA account (derived from staker collateral mint).
    #[account(
        mut,
        seeds = [b"vault", staker.collateral_mint.as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, VaultAccount>>,
    /// The vault token account from which liquidity is drawn.
    #[account(mut)]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,
    /// The staker record for collateralized borrowing.
    #[account(
        mut,
        seeds = [b"staker", borrower.key().as_ref(), staker.collateral_mint.as_ref()],
        bump
    )]
    pub staker: Box<Account<'info, Staker>>,
    /// A new loan record.
    #[account(init, payer = borrower, space = Loan::LEN)]
    pub loan: Box<Account<'info, Loan>>,
    /// The governance account.
    pub governance: Box<Account<'info, Governance>>,
    /// Global reward pool account.
    #[account(mut)]
    pub reward_pool: Box<Account<'info, RewardPool>>,
    /// The callback program to be invoked after funds transfer.
    pub callback_program: AccountInfo<'info>,
    /// The Pyth oracle price account.
    pub pyth_price: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Repay<'info> {
    /// The borrower repaying the loan.
    #[account(mut)]
    pub borrower: Signer<'info>,
    /// The borrower's token account (source of repayment funds).
    #[account(mut)]
    pub borrower_token_account: Box<Account<'info, TokenAccount>>,
    /// The vault PDA account.
    #[account(
        mut,
        seeds = [b"vault", staker.collateral_mint.as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, VaultAccount>>,
    /// The vault token account to receive the repayment.
    #[account(mut)]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,
    /// The loan record being repaid (will be closed on success).
    #[account(mut, close = borrower)]
    pub loan: Box<Account<'info, Loan>>,
    /// The staker record.
    pub staker: Box<Account<'info, Staker>>,
    /// The governance account.
    pub governance: Box<Account<'info, Governance>>,
    /// Global reward pool account.
    #[account(mut)]
    pub reward_pool: Box<Account<'info, RewardPool>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Liquidate<'info> {
    /// The liquidator.
    #[account(mut)]
    pub liquidator: Signer<'info>,
    /// The liquidator's token account to receive collateral.
    #[account(mut)]
    pub liquidator_token_account: Box<Account<'info, TokenAccount>>,
    /// The vault PDA account.
    #[account(
        mut,
        seeds = [b"vault", staker.collateral_mint.as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, VaultAccount>>,
    /// The vault token account (holds staked collateral).
    #[account(mut)]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,
    /// The loan record to be liquidated.
    #[account(mut)]
    pub loan: Box<Account<'info, Loan>>,
    /// The staker record.
    pub staker: Box<Account<'info, Staker>>,
    /// The governance account.
    pub governance: Box<Account<'info, Governance>>,
    /// Global reward pool account.
    #[account(mut)]
    pub reward_pool: Box<Account<'info, RewardPool>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CompoundRewards<'info> {
    /// The staker record.
    #[account(mut, seeds = [b"staker", staker_owner.key().as_ref(), staker.collateral_mint.as_ref()], bump)]
    pub staker: Box<Account<'info, Staker>>,
    /// The owner of the staker record.
    pub staker_owner: Signer<'info>,
    /// The governance account.
    pub governance: Box<Account<'info, Governance>>,
    /// Global reward pool account.
    #[account(mut)]
    pub reward_pool: Box<Account<'info, RewardPool>>,
}

#[derive(Accounts)]
pub struct Unstake<'info> {
    /// The user unstaking collateral.
    #[account(mut)]
    pub user: Signer<'info>,
    /// The user's token account to receive collateral.
    #[account(mut)]
    pub user_token_account: Box<Account<'info, TokenAccount>>,
    /// The vault PDA account.
    #[account(
        mut,
        seeds = [b"vault", staker.collateral_mint.as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, VaultAccount>>,
    /// The vault token account from which collateral is withdrawn.
    #[account(mut)]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,
    /// The staker record.
    #[account(mut, seeds = [b"staker", user.key().as_ref(), staker.collateral_mint.as_ref()], bump)]
    pub staker: Box<Account<'info, Staker>>,
    /// Global reward pool account.
    #[account(mut)]
    pub reward_pool: Box<Account<'info, RewardPool>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct UpdateGovernanceParameters<'info> {
    /// Only the admin (as stored in the Governance account) can update parameters.
    #[account(mut, signer, address = governance.admin)]
    pub admin: AccountInfo<'info>,
    #[account(mut)]
    pub governance: Box<Account<'info, Governance>>,
}

//
// Data Accounts
//

#[account]
pub struct VaultAccount {
    pub bump: u8,
}

#[account]
pub struct MintWrapper {
    pub bump: u8,
}

/// The loan record, with slot‑based timing and a reentrancy flag.
#[account]
pub struct Loan {
    pub borrower: Pubkey,
    pub amount: u64,
    pub start_slot: u64,
    pub due_slot: u64,
    pub active: bool,
}

impl Loan {
    // 8 + 32 + 8 + 8 + 8 + 1 = 65 bytes.
    pub const LEN: usize = 8 + 32 + 8 + 8 + 8 + 1;
}

/// Governance parameters for the protocol.
#[account]
pub struct Governance {
    pub admin: Pubkey,
    pub flash_loan_fee_bps: u64,         // default fee (unused in dynamic mode)
    pub liquidation_penalty_bps: u64,      // penalty fee per overdue slot (in basis points)
    pub liquidation_grace_slots: u64,      // grace period (in slots)
    pub compound_rate_numerator: u64,      // for auto-compounding rewards
    pub compound_rate_denominator: u64,    // for auto-compounding rewards
    pub max_borrow_ratio: u64,             // maximum borrowable amount as a percentage (in basis points) of collateral
    pub supported_collaterals: Vec<Pubkey>,// list of approved collateral mints
}

impl Governance {
    // For example, assuming up to 10 supported collaterals.
    pub const LEN: usize = 8 + 32 + (6 * 8) + 4 + (32 * 10);
}

/// Global reward pool tracking staked collateral, accrued fees, active loans, and an update counter.
#[account]
pub struct RewardPool {
    pub total_staked: u64,
    pub accrued_fees: u64,
    pub active_loan_total: u64,
    pub update_counter: u64,
}

impl RewardPool {
    // 8 + 8 + 8 + 8 = 32 bytes plus discriminator = 40 bytes total.
    pub const LEN: usize = 8 + 8 + 8 + 8;
}

/// Record for an individual staker.
#[account]
pub struct Staker {
    pub staked_amount: u64,
    pub collateral_mint: Pubkey,
    pub last_compound_slot: u64,
    pub lock_end_slot: u64,
}

impl Staker {
    // 8 + 8 + 32 + 8 + 8 = 64 bytes.
    pub const LEN: usize = 8 + 8 + 32 + 8 + 8;
}

//
// Custom Errors
//

#[error_code]
pub enum CustomError {
    #[msg("Insufficient repayment amount or fee missing.")]
    RepaymentFeeMissing,
    #[msg("Invalid collateral mint provided.")]
    InvalidCollateralMint,
    #[msg("Unsupported collateral type.")]
    UnsupportedCollateral,
    #[msg("Borrow amount exceeds allowed collateral ratio.")]
    BorrowAmountExceedsCollateral,
    #[msg("Staking is still locked.")]
    StakingLocked,
    #[msg("Insufficient staked amount.")]
    InsufficientStakedAmount,
    #[msg("Reentrancy detected.")]
    ReentrancyDetected,
    #[msg("Loan is not active.")]
    LoanNotActive,
    #[msg("Loan is not overdue for liquidation.")]
    LoanNotOverdue,
    #[msg("Oracle price unavailable.")]
    OraclePriceUnavailable,
    #[msg("Invalid timestamp: negative value encountered.")]
    InvalidTimestamp,
}
