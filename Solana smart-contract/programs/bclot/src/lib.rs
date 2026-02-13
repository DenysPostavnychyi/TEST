use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

declare_id!("4JVMVPbeQ99TT3Jz3toaiLrTNf72iZ4X8jXYj5FseExc");

// Constants
const MAX_PLAYERS_PER_ROUND: usize = 500;
const MAX_TICKETS_PER_PLAYER: u32 = 5;
const TICKET_PRICE_BTC_SATOSHIS: u64 = 5000; // 0.00005 BTC in satoshis
const BTC_DECIMALS: u8 = 8;
const USD_DECIMALS: u8 = 6;
const SECONDS_IN_DAY: i64 = 86400;
const NY_OFFSET: i64 = 4 * 3600; // UTC-4
const ROUND_DURATION: i64 = 900; // 15 minutes

#[program]
pub mod lottery {
    use super::*;

    pub fn initialize_lottery(
        ctx: Context<InitializeLottery>,
        entrance_fee_percentage: u8,
        beneficiary: Pubkey,
    ) -> Result<()> {
        require!(entrance_fee_percentage <= 20, LotteryError::InvalidEntranceFee);
        
        let lottery_state = &mut ctx.accounts.lottery_state;
        lottery_state.authority = ctx.accounts.authority.key();
        lottery_state.entrance_fee_percentage = entrance_fee_percentage;
        lottery_state.beneficiary = beneficiary;
        lottery_state.supported_tokens = Vec::new();
        lottery_state.last_timestamp = Clock::get()?.unix_timestamp;
        lottery_state.has_active_lottery = false;
        
        Ok(())
    }

    pub fn add_supported_token(
        ctx: Context<AddSupportedToken>,
        token_mint: Pubkey,
        price_feed: Pubkey,
    ) -> Result<()> {
        let lottery_state = &mut ctx.accounts.lottery_state;
        
        // Check if token already supported
        for supported in &lottery_state.supported_tokens {
            require!(supported.mint != token_mint, LotteryError::TokenAlreadySupported);
        }
        
        lottery_state.supported_tokens.push(SupportedToken {
            mint: token_mint,
            price_feed,
        });
        
        Ok(())
    }

    pub fn buy_tickets_spl(
        ctx: Context<BuyTicketsSpl>,
        token_mint: Pubkey,
        count: u32,
    ) -> Result<()> {
        require!(count > 0 && count <= MAX_TICKETS_PER_PLAYER, LotteryError::InvalidTicketCount);
        
        let lottery_state = &mut ctx.accounts.lottery_state;
        let clock = Clock::get()?;
        
        // Find supported token
        let _supported_token = lottery_state.supported_tokens
            .iter()
            .find(|t| t.mint == token_mint)
            .ok_or(LotteryError::TokenNotSupported)?;
        
        // Get or create current round
        let current_round_id = get_or_create_round(
            &mut ctx.accounts.token_lottery,
            lottery_state,
            clock.unix_timestamp,
        )?;
        
        // Calculate ticket price in token
        let ticket_price = calculate_ticket_price_for_token(
            &ctx.accounts.btc_price_feed,
            &ctx.accounts.token_price_feed,
            ctx.accounts.token_mint.decimals,
        )?;
        
        let total_cost = ticket_price * count as u64;
        
        // Transfer tokens from player to vault
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.player_token_account.to_account_info(),
                    to: ctx.accounts.vault_token_account.to_account_info(),
                    authority: ctx.accounts.player.to_account_info(),
                },
            ),
            total_cost,
        )?;
        
        // Add tickets to round
        add_tickets_to_round(
            &mut ctx.accounts.token_lottery,
            &mut ctx.accounts.player_data,
            current_round_id,
            count,
            ticket_price,
            ctx.accounts.player.key(),
            lottery_state.entrance_fee_percentage,
            clock.unix_timestamp,
        )?;
        
        emit!(TicketPurchased {
            token: token_mint,
            round_id: current_round_id as u64,
            buyer: ctx.accounts.player.key(),
            count,
            total_amount: total_cost,
            timestamp: clock.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn buy_tickets_sol(
        ctx: Context<BuyTicketsSol>,
        count: u32,
    ) -> Result<()> {
        require!(count > 0 && count <= MAX_TICKETS_PER_PLAYER, LotteryError::InvalidTicketCount);
        
        let lottery_state = &mut ctx.accounts.lottery_state;
        let clock = Clock::get()?;
        
        // Get or create current round
        let current_round_id = get_or_create_round(
            &mut ctx.accounts.sol_lottery,
            lottery_state,
            clock.unix_timestamp,
        )?;
        
        // Calculate ticket price in SOL
        let ticket_price = calculate_ticket_price_for_sol(
            &ctx.accounts.btc_price_feed,
            &ctx.accounts.sol_price_feed,
        )?;
        
        let total_cost = ticket_price * count as u64;
        
        // Verify payment amount
        require!(
            ctx.accounts.player.lamports() >= total_cost,
            LotteryError::InsufficientFunds
        );
        
        // Transfer SOL from player to vault
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.player.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                },
            ),
            total_cost,
        )?;
        
        // Add tickets to round
        add_tickets_to_round(
            &mut ctx.accounts.sol_lottery,
            &mut ctx.accounts.player_data,
            current_round_id,
            count,
            ticket_price,
            ctx.accounts.player.key(),
            lottery_state.entrance_fee_percentage,
            clock.unix_timestamp,
        )?;
        
        emit!(TicketPurchased {
            token: Pubkey::default(), // SOL represented as default pubkey
            round_id: current_round_id as u64,
            buyer: ctx.accounts.player.key(),
            count,
            total_amount: total_cost,
            timestamp: clock.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn close_round_and_pick_winner(
        ctx: Context<CloseRound>,
        token_mint: Option<Pubkey>,
        randomness: u64, // In production, this would come from VRF
    ) -> Result<()> {
        let lottery = match token_mint {
            Some(_) => &mut ctx.accounts.token_lottery,
            None => &mut ctx.accounts.sol_lottery,
        };
        
        require!(!lottery.rounds.is_empty(), LotteryError::NoActiveRound);
        
        let current_round_id = lottery.rounds.len() - 1;
        let round = &mut lottery.rounds[current_round_id];
        
        require!(round.status == RoundStatus::Open, LotteryError::RoundNotOpen);
        require!(!round.tickets.is_empty(), LotteryError::NoTicketsInRound);
        
        // Pick winner using randomness
        let winner_ticket_index = (randomness as usize) % round.tickets.len();
        let winning_ticket = &round.tickets[winner_ticket_index];
        
        round.winner_address = Some(winning_ticket.owner);
        round.winner_ticket_index = Some(winner_ticket_index as u32);
        round.status = RoundStatus::Closed;
        
        // Calculate prize and commission
        let commission = (round.pool_balance * ctx.accounts.lottery_state.entrance_fee_percentage as u64) / 100;
        let prize_amount = round.pool_balance - commission;
        
        round.commission_balance = commission;
        round.pool_balance = prize_amount;
        
        emit!(WinnerPicked {
            token: token_mint.unwrap_or_default(),
            round_id: current_round_id as u64,
            winner: winning_ticket.owner,
            prize_amount,
            timestamp: Clock::get()?.unix_timestamp,
        });
        
        Ok(())
    }

    pub fn claim_prize_sol(ctx: Context<ClaimPrizeSol>, round_id: u64) -> Result<()> {
        let lottery = &mut ctx.accounts.sol_lottery;
        let round = &mut lottery.rounds[round_id as usize];
        
        require!(round.status == RoundStatus::Closed, LotteryError::RoundNotClosed);
        require!(round.winner_address == Some(ctx.accounts.winner.key()), LotteryError::NotTheWinner);
        require!(!round.prize_claimed, LotteryError::PrizeAlreadyClaimed);
        
        round.prize_claimed = true;
        
        // Transfer prize to winner
        **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= round.pool_balance;
        **ctx.accounts.winner.to_account_info().try_borrow_mut_lamports()? += round.pool_balance;
        
        // Transfer commission to beneficiary
        **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= round.commission_balance;
        **ctx.accounts.beneficiary.to_account_info().try_borrow_mut_lamports()? += round.commission_balance;
        
        emit!(PrizeClaimed {
            token: Pubkey::default(),
            round_id,
            winner: ctx.accounts.winner.key(),
            amount: round.pool_balance,
        });
        
        Ok(())
    }

    pub fn claim_prize_spl(
        ctx: Context<ClaimPrizeSpl>,
        token_mint: Pubkey,
        round_id: u64,
    ) -> Result<()> {
        let lottery = &mut ctx.accounts.token_lottery;
        let round = &mut lottery.rounds[round_id as usize];
        
        require!(round.status == RoundStatus::Closed, LotteryError::RoundNotClosed);
        require!(round.winner_address == Some(ctx.accounts.winner.key()), LotteryError::NotTheWinner);
        require!(!round.prize_claimed, LotteryError::PrizeAlreadyClaimed);
        
        round.prize_claimed = true;
        
        // Transfer prize to winner
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.winner_token_account.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                &[&[
                    b"vault",
                    token_mint.as_ref(),
                    &[ctx.bumps.vault_authority],
                ]],
            ),
            round.pool_balance,
        )?;
        
        // Transfer commission to beneficiary
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.beneficiary_token_account.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                &[&[
                    b"vault",
                    token_mint.as_ref(),
                    &[ctx.bumps.vault_authority],
                ]],
            ),
            round.commission_balance,
        )?;
        
        emit!(PrizeClaimed {
            token: token_mint,
            round_id,
            winner: ctx.accounts.winner.key(),
            amount: round.pool_balance,
        });
        
        Ok(())
    }
}

// Helper functions
fn get_or_create_round(
    lottery: &mut Account<TokenLottery>,
    lottery_state: &mut Account<LotteryState>,
    current_timestamp: i64,
) -> Result<usize> {
    // Check if there's an active round
    if let Some(last_round) = lottery.rounds.last() {
        if last_round.status == RoundStatus::Open && current_timestamp < last_round.end_time {
            return Ok(lottery.rounds.len() - 1);
        }
    }
    
    // Create new round
    let round_id = lottery.rounds.len();
    lottery.rounds.push(Round {
        status: RoundStatus::Open,
        start_time: current_timestamp,
        end_time: get_temporary_close_time(current_timestamp),
        pool_balance: 0,
        commission_balance: 0,
        tickets: Vec::new(),
        winner_address: None,
        winner_ticket_index: None,
        prize_claimed: false,
    });
    
    lottery_state.last_timestamp = current_timestamp;
    lottery_state.has_active_lottery = true;
    
    Ok(round_id)
}

fn get_temporary_close_time(current_timestamp: i64) -> i64 {
    let round_active_time = current_timestamp % ROUND_DURATION;
    current_timestamp - round_active_time + ROUND_DURATION
}

fn add_tickets_to_round(
    lottery: &mut Account<TokenLottery>,
    player_data: &mut Account<PlayerData>,
    round_id: usize,
    count: u32,
    ticket_price: u64,
    player: Pubkey,
    entrance_fee_percentage: u8,
    timestamp: i64,
) -> Result<()> {
    let round = &mut lottery.rounds[round_id];
    
    // Calculate costs
    let total_cost = ticket_price * count as u64;
    let commission = (total_cost * entrance_fee_percentage as u64) / 100;
    let pool_amount = total_cost - commission;
    
    round.pool_balance += pool_amount;
    round.commission_balance += commission;
    
    // Check if this is the first buyer (for bonus ticket)
    let is_first_buyer = round.tickets.is_empty();
    
    // Add regular tickets
    for _ in 0..count {
        round.tickets.push(Ticket {
            owner: player,
            price: ticket_price,
            timestamp,
            is_bonus: false,
        });
    }
    
    // Add bonus ticket for first buyer
    if is_first_buyer {
        round.tickets.push(Ticket {
            owner: player,
            price: 0,
            timestamp,
            is_bonus: true,
        });
        
        player_data.has_bonus_ticket = true;
        
        emit!(FirstTicketBonusAwarded {
            round_id: round_id as u64,
            buyer: player,
            timestamp,
        });
    }
    
    player_data.tickets_count += count + if is_first_buyer { 1 } else { 0 };
    
    Ok(())
}

fn calculate_ticket_price_for_sol(
    _btc_price_feed: &AccountInfo,
    _sol_price_feed: &AccountInfo,
) -> Result<u64> {
    // This would integrate with Switchboard or Pyth oracles
    // For now, returning a placeholder
    // In production, you'd read from the price feeds
    Ok(25_000_000) // ~0.025 SOL placeholder
}

fn calculate_ticket_price_for_token(
    _btc_price_feed: &AccountInfo,
    _token_price_feed: &AccountInfo,
    _token_decimals: u8,
) -> Result<u64> {
    // This would integrate with Switchboard or Pyth oracles
    // For now, returning a placeholder
    // In production, you'd read from the price feeds
    Ok(5_000_000) // Placeholder based on token decimals
}

// Account validation structs
#[derive(Accounts)]
pub struct InitializeLottery<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + LotteryState::INIT_SPACE,
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, LotteryState>,
    
    #[account(mut)]
    pub authority: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AddSupportedToken<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump,
        has_one = authority
    )]
    pub lottery_state: Account<'info, LotteryState>,
    
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct BuyTicketsSpl<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, LotteryState>,
    
    #[account(
        init_if_needed,
        payer = player,
        space = 8 + TokenLottery::INIT_SPACE,
        seeds = [b"token_lottery", token_mint.key().as_ref()],
        bump
    )]
    pub token_lottery: Account<'info, TokenLottery>,
    
    #[account(
        init_if_needed,
        payer = player,
        space = 8 + PlayerData::INIT_SPACE,
        seeds = [b"player_data", player.key().as_ref(), token_mint.key().as_ref()],
        bump
    )]
    pub player_data: Account<'info, PlayerData>,
    
    #[account(mut)]
    pub player: Signer<'info>,
    
    pub token_mint: Account<'info, anchor_spl::token::Mint>,
    
    #[account(mut)]
    pub player_token_account: Account<'info, TokenAccount>,
    
    #[account(
        init_if_needed,
        payer = player,
        token::mint = token_mint,
        token::authority = vault_authority,
        seeds = [b"vault", token_mint.key().as_ref()],
        bump
    )]
    pub vault_token_account: Account<'info, TokenAccount>,
    
    #[account(
        seeds = [b"vault_authority", token_mint.key().as_ref()],
        bump
    )]
    /// CHECK: PDA authority for vault
    pub vault_authority: AccountInfo<'info>,
    
    /// CHECK: BTC price feed account
    pub btc_price_feed: AccountInfo<'info>,
    
    /// CHECK: Token price feed account
    pub token_price_feed: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BuyTicketsSol<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump
    )]
    pub lottery_state: Account<'info, LotteryState>,
    
    #[account(
        init_if_needed,
        payer = player,
        space = 8 + TokenLottery::INIT_SPACE,
        seeds = [b"sol_lottery"],
        bump
    )]
    pub sol_lottery: Account<'info, TokenLottery>,
    
    #[account(
        init_if_needed,
        payer = player,
        space = 8 + PlayerData::INIT_SPACE,
        seeds = [b"player_data", player.key().as_ref(), b"SOL"],
        bump
    )]
    pub player_data: Account<'info, PlayerData>,
    
    #[account(mut)]
    pub player: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"sol_vault"],
        bump
    )]
    /// CHECK: SOL vault PDA
    pub vault: AccountInfo<'info>,
    
    /// CHECK: BTC price feed account
    pub btc_price_feed: AccountInfo<'info>,
    
    /// CHECK: SOL price feed account
    pub sol_price_feed: AccountInfo<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CloseRound<'info> {
    #[account(
        mut,
        seeds = [b"lottery_state"],
        bump,
        has_one = authority
    )]
    pub lottery_state: Account<'info, LotteryState>,
    
    #[account(mut)]
    pub token_lottery: Account<'info, TokenLottery>,
    
    #[account(mut)]
    pub sol_lottery: Account<'info, TokenLottery>,
    
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct ClaimPrizeSol<'info> {
    #[account(mut)]
    pub sol_lottery: Account<'info, TokenLottery>,
    
    #[account(mut)]
    pub winner: Signer<'info>,
    
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
pub struct ClaimPrizeSpl<'info> {
    #[account(mut)]
    pub token_lottery: Account<'info, TokenLottery>,
    
    #[account(mut)]
    pub winner: Signer<'info>,
    
    #[account(mut)]
    pub vault_token_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub winner_token_account: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub beneficiary_token_account: Account<'info, TokenAccount>,
    
    #[account(
        seeds = [b"vault_authority", token_lottery.key().as_ref()],
        bump
    )]
    /// CHECK: PDA authority for vault
    pub vault_authority: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token>,
}

// Data structures
#[account]
#[derive(InitSpace)]
pub struct LotteryState {
    pub authority: Pubkey,
    pub entrance_fee_percentage: u8,
    pub beneficiary: Pubkey,
    #[max_len(10)]
    pub supported_tokens: Vec<SupportedToken>,
    pub last_timestamp: i64,
    pub has_active_lottery: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace)]
pub struct SupportedToken {
    pub mint: Pubkey,
    pub price_feed: Pubkey,
}

#[account]
#[derive(InitSpace)]
pub struct TokenLottery {
    #[max_len(1000)]
    pub rounds: Vec<Round>,
}

#[account]
#[derive(InitSpace)]
pub struct PlayerData {
    pub tickets_count: u32,
    pub has_bonus_ticket: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace)]
pub struct Round {
    pub status: RoundStatus,
    pub start_time: i64,
    pub end_time: i64,
    pub pool_balance: u64,
    pub commission_balance: u64,
    #[max_len(5000)]
    pub tickets: Vec<Ticket>,
    pub winner_address: Option<Pubkey>,
    pub winner_ticket_index: Option<u32>,
    pub prize_claimed: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace)]
pub struct Ticket {
    pub owner: Pubkey,
    pub price: u64,
    pub timestamp: i64,
    pub is_bonus: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace)]
pub enum RoundStatus {
    Open,
    Closed,
}

// Events
#[event]
pub struct TicketPurchased {
    pub token: Pubkey,
    pub round_id: u64,
    pub buyer: Pubkey,
    pub count: u32,
    pub total_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct FirstTicketBonusAwarded {
    pub round_id: u64,
    pub buyer: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct WinnerPicked {
    pub token: Pubkey,
    pub round_id: u64,
    pub winner: Pubkey,
    pub prize_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct PrizeClaimed {
    pub token: Pubkey,
    pub round_id: u64,
    pub winner: Pubkey,
    pub amount: u64,
}

// Error codes
#[error_code]
pub enum LotteryError {
    #[msg("Invalid entrance fee percentage")]
    InvalidEntranceFee,
    #[msg("Token already supported")]
    TokenAlreadySupported,
    #[msg("Token not supported")]
    TokenNotSupported,
    #[msg("Invalid ticket count")]
    InvalidTicketCount,
    #[msg("Insufficient funds")]
    InsufficientFunds,
    #[msg("No active round")]
    NoActiveRound,
    #[msg("Round not open")]
    RoundNotOpen,
    #[msg("No tickets in round")]
    NoTicketsInRound,
    #[msg("Round not closed")]
    RoundNotClosed,
    #[msg("Not the winner")]
    NotTheWinner,
    #[msg("Prize already claimed")]
    PrizeAlreadyClaimed,
}