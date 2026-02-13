// vrf.rs - Switchboard VRF integration for verifiable randomness
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct RequestRandomness<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump,
        has_one = authority
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
    
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + VrfRequest::INIT_SPACE,
        seeds = [b"vrf_request", &lottery_state.round_counter.to_le_bytes()],
        bump
    )]
    pub vrf_request: Account<'info, VrfRequest>,
    
    pub authority: Signer<'info>,
    
    // Switchboard VRF accounts
    #[account(
        seeds = [b"vrf_auth"],
        bump
    )]
    /// CHECK: VRF authority PDA
    pub vrf_auth: AccountInfo<'info>,
    
    #[account(mut)]
    /// CHECK: Switchboard VRF account
    pub vrf: AccountInfo<'info>,
    
    #[account(mut)]
    /// CHECK: Oracle queue
    pub oracle_queue: AccountInfo<'info>,
    
    /// CHECK: Queue authority
    pub queue_authority: AccountInfo<'info>,
    
    /// CHECK: Data buffer
    pub data_buffer: AccountInfo<'info>,
    
    #[account(mut)]
    /// CHECK: Permission account
    pub permission: AccountInfo<'info>,
    
    #[account(mut)]
    /// CHECK: Escrow account
    pub escrow: AccountInfo<'info>,
    
    /// CHECK: Recent blockhashes sysvar
    pub recent_blockhashes: AccountInfo<'info>,
    
    /// CHECK: Switchboard program state
    pub program_state: AccountInfo<'info>,
    
    /// CHECK: Switchboard program
    pub switchboard_program: AccountInfo<'info>,
    
    pub token_program: Program<'info, anchor_spl::token::Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ConsumeRandomness<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, crate::LotteryState>,
    
    #[account(
        mut,
        seeds = [b"vrf_request", &vrf_request.round_id.to_le_bytes()],
        bump
    )]
    pub vrf_request: Account<'info, VrfRequest>,
    
    #[account(mut)]
    pub token_lottery: Option<Account<'info, crate::TokenLottery>>,
    
    #[account(mut)]
    pub sol_lottery: Option<Account<'info, crate::TokenLottery>>,
    
    #[account(mut)]
    /// CHECK: Switchboard VRF account
    pub vrf: AccountInfo<'info>,
}

#[account]
#[derive(InitSpace)]
pub struct VrfRequest {
    pub round_id: u64,
    pub token_mint: Option<Pubkey>,
    pub requested_at: i64,
    pub fulfilled: bool,
    pub randomness: Option<[u8; 32]>,
}

// VRF instruction implementations
impl<'info> RequestRandomness<'info> {
    pub fn request_randomness_for_round(
        &mut self,
        token_mint: Option<Pubkey>,
        round_id: u64,
    ) -> Result<()> {
        let lottery_state = &mut self.lottery_state;
        let vrf_request = &mut self.vrf_request;
        
        vrf_request.round_id = round_id;
        vrf_request.token_mint = token_mint;
        vrf_request.requested_at = Clock::get()?.unix_timestamp;
        vrf_request.fulfilled = false;
        vrf_request.randomness = None;
        
        // TODO: Implement actual Switchboard VRF request
        // This would use the Switchboard SDK to request randomness
        // For now, we'll mark as requested and use pseudo-randomness
        
        msg!("VRF randomness requested for round {} of token {:?}", round_id, token_mint);
        
        Ok(())
    }
}

impl<'info> ConsumeRandomness<'info> {
    pub fn consume_randomness_and_pick_winner(&mut self) -> Result<()> {
        let vrf_request = &mut self.vrf_request;
        
        require!(!vrf_request.fulfilled, crate::LotteryError::RandomnessAlreadyConsumed);
        
        // In production, this would read from the Switchboard VRF account
        // For now, using timestamp-based pseudo-randomness
        let clock = Clock::get()?;
        let pseudo_randomness = ((clock.slot as u128)
            .wrapping_mul(clock.unix_timestamp as u128)
            .wrapping_mul(vrf_request.round_id as u128)) as u64;
        
        vrf_request.fulfilled = true;
        
        // Pick winner based on randomness
        match vrf_request.token_mint {
            Some(token_mint) => {
                if let Some(lottery) = &mut self.token_lottery {
                    self.pick_winner_for_lottery(lottery, pseudo_randomness)?;
                }
            }
            None => {
                if let Some(lottery) = &mut self.sol_lottery {
                    self.pick_winner_for_lottery(lottery, pseudo_randomness)?;
                }
            }
        }
        
        Ok(())
    }
    
    fn pick_winner_for_lottery(
        &self,
        lottery: &mut Account<crate::TokenLottery>,
        randomness: u64,
    ) -> Result<()> {
        let round_id = self.vrf_request.round_id as usize;
        require!(round_id < lottery.rounds.len(), crate::LotteryError::InvalidRoundId);
        
        let round = &mut lottery.rounds[round_id];
        require!(round.status == crate::RoundStatus::Open, crate::LotteryError::RoundNotOpen);
        require!(!round.tickets.is_empty(), crate::LotteryError::NoTicketsInRound);
        
        // Pick winner
        let winner_ticket_index = (randomness as usize) % round.tickets.len();
        let winning_ticket = &round.tickets[winner_ticket_index];
        
        round.winner_address = Some(winning_ticket.owner);
        round.winner_ticket_index = Some(winner_ticket_index as u32);
        round.status = crate::RoundStatus::Closed;
        
        emit!(crate::WinnerPicked {
            token: self.vrf_request.token_mint.unwrap_or_default(),
            round_id: round_id as u64,
            winner: winning_ticket.owner,
            prize_amount: round.pool_balance,
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }
}

// VRF callback function (called by Switchboard)
#[derive(Accounts)]
pub struct VrfCallback<'info> {
    #[account(
        mut,
        seeds = [b"vrf_request", &vrf_request.round_id.to_le_bytes()],
        bump
    )]
    pub vrf_request: Account<'info, VrfRequest>,
    
    #[account(
        seeds = [b"vrf_auth"],
        bump
    )]
    /// CHECK: VRF authority PDA
    pub vrf_auth: AccountInfo<'info>,
    
    /// CHECK: Switchboard VRF account
    pub vrf: AccountInfo<'info>,
}

pub fn vrf_callback(ctx: Context<VrfCallback>, result: [u8; 32]) -> Result<()> {
    let vrf_request = &mut ctx.accounts.vrf_request;
    
    require!(!vrf_request.fulfilled, crate::LotteryError::RandomnessAlreadyConsumed);
    
    vrf_request.randomness = Some(result);
    vrf_request.fulfilled = true;
    
    msg!("VRF callback received for round {}", vrf_request.round_id);
    
    Ok(())
}