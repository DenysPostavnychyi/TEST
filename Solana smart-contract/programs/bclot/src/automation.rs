// automation.rs - Clockwork integration for automatic round management
use anchor_lang::prelude::*;

// Automation functions similar to Chainlink Keepers
impl crate::lottery {
    /// Check if upkeep is needed (similar to Chainlink's checkUpkeep)
    pub fn check_upkeep(ctx: Context<CheckUpkeep>) -> Result<bool> {
        let lottery_state = &ctx.accounts.lottery_state;
        let clock = Clock::get()?;
        
        // Upkeep is needed if:
        // 1. Time interval has passed
        // 2. There's an active lottery
        let upkeep_needed = clock.unix_timestamp > lottery_state.last_timestamp 
            && lottery_state.has_active_lottery;
        
        Ok(upkeep_needed)
    }
    
    /// Perform upkeep (similar to Chainlink's performUpkeep)
    pub fn perform_upkeep(ctx: Context<PerformUpkeep>) -> Result<()> {
        let lottery_state = &mut ctx.accounts.lottery_state;
        let clock = Clock::get()?;
        
        // Verify upkeep is needed
        require!(
            clock.unix_timestamp > lottery_state.last_timestamp && lottery_state.has_active_lottery,
            crate::LotteryError::UpkeepNotNeeded
        );
        
        // Update state
        lottery_state.last_timestamp = get_next_round_time(clock.unix_timestamp);
        lottery_state.has_active_lottery = false;
        
        // Close all open rounds for all supported tokens
        for supported_token in &lottery_state.supported_tokens {
            // Request VRF for this token
            // In production, this would trigger VRF requests
            msg!("Requesting VRF for token: {}", supported_token.mint);
        }
        
        // Also handle SOL lottery
        msg!("Requesting VRF for SOL lottery");
        
        emit!(UpkeepPerformed {
            timestamp: clock.unix_timestamp,
            next_round_time: lottery_state.last_timestamp,
        });
        
        Ok(())
    }
    
    /// Automated round closure for specific token
    pub fn auto_close_round(
        ctx: Context<AutoCloseRound>,
        token_mint: Option<Pubkey>,
    ) -> Result<()> {
        let clock = Clock::get()?;
        
        let lottery = match token_mint {
            Some(_) => &mut ctx.accounts.token_lottery,
            None => &mut ctx.accounts.sol_lottery,
        };
        
        // Find the current open round
        if let Some(round) = lottery.rounds.last_mut() {
            require!(round.status == crate::RoundStatus::Open, crate::LotteryError::RoundNotOpen);
            require!(clock.unix_timestamp >= round.end_time, crate::LotteryError::RoundStillActive);
            
            // Mark round as ready for VRF
            round.status = crate::RoundStatus::PendingVrf;
            
            emit!(RoundReadyForVrf {
                token: token_mint.unwrap_or_default(),
                round_id: (lottery.rounds.len() - 1) as u64,
                end_time: round.end_time,
                ticket_count: round.tickets.len() as u32,
            });
        }
        
        Ok(())
    }
    
    /// Batch process multiple rounds
    pub fn batch_process_rounds(ctx: Context<BatchProcessRounds>) -> Result<()> {
        let clock = Clock::get()?;
        let lottery_state = &ctx.accounts.lottery_state;
        
        // Process SOL lottery first
        if let Some(sol_lottery) = &mut ctx.accounts.sol_lottery {
            if let Some(round) = sol_lottery.rounds.last_mut() {
                if round.status == crate::RoundStatus::Open && clock.unix_timestamp >= round.end_time {
                    round.status = crate::RoundStatus::PendingVrf;
                    
                    emit!(RoundReadyForVrf {
                        token: Pubkey::default(),
                        round_id: (sol_lottery.rounds.len() - 1) as u64,
                        end_time: round.end_time,
                        ticket_count: round.tickets.len() as u32,
                    });
                }
            }
        }
        
        // Process each supported token lottery
        for (index, supported_token) in lottery_state.supported_tokens.iter().enumerate() {
            // In a real implementation, you'd load each token lottery account
            // For this example, we'll just emit events
            msg!("Processing lottery for token {}: {}", index, supported_token.mint);
        }
        
        Ok(())
    }
}

// Helper function for calculating next round time
fn get_next_round_time(current_timestamp: i64) -> i64 {
    let round_active_time = current_timestamp % crate::ROUND_DURATION;
    current_timestamp - round_active_time + crate::ROUND_DURATION
}

// Account contexts for automation
#[derive(Accounts)]
pub struct CheckUpkeep<'info> {
    #[account(
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
}

#[derive(Accounts)]
pub struct PerformUpkeep<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
    
    /// CHECK: Clockwork thread authority
    pub thread_authority: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct AutoCloseRound<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
    
    #[account(mut)]
    pub token_lottery: Option<Account<'info, crate::TokenLottery>>,
    
    #[account(mut)]
    pub sol_lottery: Option<Account<'info, crate::TokenLottery>>,
    
    /// CHECK: Clockwork thread authority
    pub thread_authority: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct BatchProcessRounds<'info> {
    #[account(
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
    
    #[account(mut)]
    pub sol_lottery: Option<Account<'info, crate::TokenLottery>>,
    
    /// CHECK: Clockwork thread authority
    pub thread_authority: AccountInfo<'info>,
}

// Enhanced round status
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace)]
pub enum EnhancedRoundStatus {
    Open,
    PendingVrf,
    Closed,
}

// Additional events for automation
#[event]
pub struct UpkeepPerformed {
    pub timestamp: i64,
    pub next_round_time: i64,
}

#[event]
pub struct RoundReadyForVrf {
    pub token: Pubkey,
    pub round_id: u64,
    pub end_time: i64,
    pub ticket_count: u32,
}

// Additional error codes for automation
impl crate::LotteryError {
    pub const UpkeepNotNeeded: crate::LotteryError = crate::LotteryError::UpkeepNotNeeded;
    pub const RoundStillActive: crate::LotteryError = crate::LotteryError::RoundStillActive;
    pub const RandomnessAlreadyConsumed: crate::LotteryError = crate::LotteryError::RandomnessAlreadyConsumed;
    pub const InvalidRoundId: crate::LotteryError = crate::LotteryError::InvalidRoundId;
}