// admin.rs - Administrative functions and view functions
use anchor_lang::prelude::*;

// Admin instruction implementations
impl crate::lottery {
    pub fn update_entrance_fee(
        ctx: Context<UpdateConfig>,
        new_fee_percentage: u8,
    ) -> Result<()> {
        require!(new_fee_percentage <= 20, crate::LotteryError::InvalidEntranceFee);
        
        ctx.accounts.lottery_state.entrance_fee_percentage = new_fee_percentage;
        
        emit!(ConfigUpdated {
            field: "entrance_fee_percentage".to_string(),
            old_value: ctx.accounts.lottery_state.entrance_fee_percentage as u64,
            new_value: new_fee_percentage as u64,
        });
        
        Ok(())
    }
    
    pub fn update_beneficiary(
        ctx: Context<UpdateConfig>,
        new_beneficiary: Pubkey,
    ) -> Result<()> {
        let old_beneficiary = ctx.accounts.lottery_state.beneficiary;
        ctx.accounts.lottery_state.beneficiary = new_beneficiary;
        
        emit!(BeneficiaryUpdated {
            old_beneficiary,
            new_beneficiary,
        });
        
        Ok(())
    }
    
    pub fn emergency_pause(ctx: Context<UpdateConfig>) -> Result<()> {
        ctx.accounts.lottery_state.has_active_lottery = false;
        
        emit!(EmergencyPause {
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }
    
    pub fn withdraw_commission_sol(
        ctx: Context<WithdrawCommissionSol>,
        amount: u64,
    ) -> Result<()> {
        let vault_lamports = ctx.accounts.vault.lamports();
        require!(vault_lamports >= amount, crate::LotteryError::InsufficientFunds);
        
        **ctx.accounts.vault.try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.beneficiary.try_borrow_mut_lamports()? += amount;
        
        emit!(CommissionWithdrawn {
            token: Pubkey::default(),
            amount,
            beneficiary: ctx.accounts.beneficiary.key(),
        });
        
        Ok(())
    }
    
    pub fn withdraw_commission_spl(
        ctx: Context<WithdrawCommissionSpl>,
        amount: u64,
    ) -> Result<()> {
        anchor_spl::token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                anchor_spl::token::Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.beneficiary_token_account.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                &[&[
                    b"vault_authority",
                    ctx.accounts.token_mint.key().as_ref(),
                    &[ctx.bumps.vault_authority],
                ]],
            ),
            amount,
        )?;
        
        emit!(CommissionWithdrawn {
            token: ctx.accounts.token_mint.key(),
            amount,
            beneficiary: ctx.accounts.beneficiary_token_account.owner,
        });
        
        Ok(())
    }
    
    // View functions
    pub fn get_lottery_state(ctx: Context<GetLotteryState>) -> Result<LotteryStateView> {
        let state = &ctx.accounts.lottery_state;
        
        Ok(LotteryStateView {
            authority: state.authority,
            entrance_fee_percentage: state.entrance_fee_percentage,
            beneficiary: state.beneficiary,
            supported_tokens: state.supported_tokens.clone(),
            last_timestamp: state.last_timestamp,
            has_active_lottery: state.has_active_lottery,
        })
    }
    
    pub fn get_round_data(
        ctx: Context<GetRoundData>,
        round_id: u64,
    ) -> Result<RoundView> {
        let lottery = &ctx.accounts.lottery;
        let round_index = round_id as usize;
        
        require!(round_index < lottery.rounds.len(), crate::LotteryError::InvalidRoundId);
        
        let round = &lottery.rounds[round_index];
        
        Ok(RoundView {
            round_id,
            status: round.status.clone(),
            start_time: round.start_time,
            end_time: round.end_time,
            pool_balance: round.pool_balance,
            commission_balance: round.commission_balance,
            ticket_count: round.tickets.len() as u32,
            winner_address: round.winner_address,
            winner_ticket_index: round.winner_ticket_index,
            prize_claimed: round.prize_claimed,
        })
    }
    
    pub fn get_player_tickets(
        ctx: Context<GetPlayerTickets>,
        round_id: u64,
        player: Pubkey,
    ) -> Result<PlayerTicketsView> {
        let lottery = &ctx.accounts.lottery;
        let round_index = round_id as usize;
        
        require!(round_index < lottery.rounds.len(), crate::LotteryError::InvalidRoundId);
        
        let round = &lottery.rounds[round_index];
        let mut ticket_indices = Vec::new();
        let mut total_tickets = 0u32;
        let mut has_bonus = false;
        
        for (index, ticket) in round.tickets.iter().enumerate() {
            if ticket.owner == player {
                ticket_indices.push(index as u32);
                total_tickets += 1;
                if ticket.is_bonus {
                    has_bonus = true;
                }
            }
        }
        
        Ok(PlayerTicketsView {
            player,
            round_id,
            ticket_indices,
            total_tickets,
            has_bonus_ticket: has_bonus,
        })
    }
    
    pub fn get_current_ticket_price_sol(
        ctx: Context<GetTicketPrice>,
    ) -> Result<u64> {
        crate::price_feeds::calculate_ticket_price_in_lamports(
            &ctx.accounts.btc_price_feed,
            &ctx.accounts.sol_price_feed,
        )
    }
    
    pub fn get_current_ticket_price_spl(
        ctx: Context<GetTicketPriceSpl>,
    ) -> Result<u64> {
        crate::price_feeds::calculate_ticket_price_in_tokens(
            &ctx.accounts.btc_price_feed,
            &ctx.accounts.token_price_feed,
            ctx.accounts.token_mint.decimals,
        )
    }
}

// Account contexts for admin functions
#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump,
        has_one = authority
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
    
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct WithdrawCommissionSol<'info> {
    #[account(
        seeds = [b"lottery_state"],
        bump,
        has_one = authority,
        has_one = beneficiary
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
    
    pub authority: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"sol_vault"],
        bump
    )]
    /// CHECK: SOL vault PDA
    pub vault: AccountInfo<'info>,
    
    #[account(mut)]
    /// CHECK: Beneficiary account
    pub beneficiary: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct WithdrawCommissionSpl<'info> {
    #[account(
        seeds = [b"lottery_state"],
        bump,
        has_one = authority
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
    
    pub authority: Signer<'info>,
    
    pub token_mint: Account<'info, anchor_spl::token::Mint>,
    
    #[account(mut)]
    pub vault_token_account: Account<'info, anchor_spl::token::TokenAccount>,
    
    #[account(mut)]
    pub beneficiary_token_account: Account<'info, anchor_spl::token::TokenAccount>,
    
    #[account(
        seeds = [b"vault_authority", token_mint.key().as_ref()],
        bump
    )]
    /// CHECK: PDA authority for vault
    pub vault_authority: AccountInfo<'info>,
    
    pub token_program: Program<'info, anchor_spl::token::Token>,
}

#[derive(Accounts)]
pub struct GetLotteryState<'info> {
    #[account(
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
}

#[derive(Accounts)]
pub struct GetRoundData<'info> {
    pub lottery: Account<'info, crate::TokenLottery>,
}

#[derive(Accounts)]
pub struct GetPlayerTickets<'info> {
    pub lottery: Account<'info, crate::TokenLottery>,
}

#[derive(Accounts)]
pub struct GetTicketPrice<'info> {
    /// CHECK: BTC price feed
    pub btc_price_feed: AccountInfo<'info>,
    /// CHECK: SOL price feed
    pub sol_price_feed: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct GetTicketPriceSpl<'info> {
    pub token_mint: Account<'info, anchor_spl::token::Mint>,
    /// CHECK: BTC price feed
    pub btc_price_feed: AccountInfo<'info>,
    /// CHECK: Token price feed
    pub token_price_feed: AccountInfo<'info>,
}

// View data structures
#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct LotteryStateView {
    pub authority: Pubkey,
    pub entrance_fee_percentage: u8,
    pub beneficiary: Pubkey,
    pub supported_tokens: Vec<crate::SupportedToken>,
    pub last_timestamp: i64,
    pub has_active_lottery: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct RoundView {
    pub round_id: u64,
    pub status: crate::RoundStatus,
    pub start_time: i64,
    pub end_time: i64,
    pub pool_balance: u64,
    pub commission_balance: u64,
    pub ticket_count: u32,
    pub winner_address: Option<Pubkey>,
    pub winner_ticket_index: Option<u32>,
    pub prize_claimed: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct PlayerTicketsView {
    pub player: Pubkey,
    pub round_id: u64,
    pub ticket_indices: Vec<u32>,
    pub total_tickets: u32,
    pub has_bonus_ticket: bool,
}

// Additional events
#[event]
pub struct ConfigUpdated {
    pub field: String,
    pub old_value: u64,
    pub new_value: u64,
}

#[event]
pub struct BeneficiaryUpdated {
    pub old_beneficiary: Pubkey,
    pub new_beneficiary: Pubkey,
}

#[event]
pub struct EmergencyPause {
    pub timestamp: i64,
}

#[event]
pub struct CommissionWithdrawn {
    pub token: Pubkey,
    pub amount: u64,
    pub beneficiary: Pubkey,
}