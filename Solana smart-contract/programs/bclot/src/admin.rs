use anchor_lang::prelude::*;
use std::collections::HashMap;
use crate::calculate_ticket_price_for_sol;
use crate::TICKET_BTC_SATOSHIS;
use crate::Round;
use crate::RoundStatus;
use crate::RaffleState;
use crate::TokenRaffle;
    
pub fn get_raffle_state(ctx: Context<GetRaffleState>) -> Result<RaffleStateView> {
    let state = &ctx.accounts.raffle_state;
    
    Ok(RaffleStateView {
        authority: state.authority,
        entrance_fee_percentage: state.entrance_fee_percentage,
        beneficiary: state.beneficiary,
        created_at: state.created_at,
        vrf_request_counter: state.vrf_request_counter,
        test_ticket_price: state.test_ticket_price,
    })
}


pub fn get_token_raffle(ctx: Context<GetTokenRaffle>) -> Result<TokenRaffleView> {
    let raffle = &ctx.accounts.sol_raffle;

    Ok(TokenRaffleView {
        authority: raffle.authority,
        token_mint: raffle.token_mint,
        current_round_id: raffle.current_round_id,
        current_round_status: raffle.current_round_status.clone(),
        current_round_end_time: raffle.current_round_end_time,
        total_rounds: raffle.total_rounds,
        pending_rounds: raffle.pending_rounds.clone(),
    })
}

pub fn get_raffle_round_result(ctx: Context<GetRoundAccounts>, round_id: u32) -> Result<RoundResultView> {

    let round = &ctx.accounts.round;

    let mut players_map: std::collections::HashMap<Pubkey, RoundPlayerDataWithAddress> = HashMap::new();

    // for ticket in round.tickets.iter() {
    //     let entry = players_map.entry(ticket.owner).or_insert(RoundPlayerDataWithAddress {
    //         player: ticket.owner,
    //         tickets_count: 0,
    //         has_bonus_ticket: false,
    //     });

    //     entry.tickets_count += 1;
    //     if ticket.is_bonus {
    //         entry.has_bonus_ticket = true;
    //     }
    // }

    let round_players: Vec<RoundPlayerDataWithAddress> = players_map.into_values().collect();

    Ok(RoundResultView {
        winner_address: round.winner_address,
        winner_ticket_index: round.winner_ticket_index,
        round_players,
        prize_amount: round.prize_amount,
    })
}

pub fn get_raffle_round_data(ctx: Context<GetRoundAccounts>, round_id: u32) -> Result<RoundDataView> {

    let round_account_info = ctx.accounts.round.to_account_info();

    if round_account_info.data_is_empty() {
        return err!(ErrorCode::RoundDoesNotExist);
    }

    let round = &ctx.accounts.round;

    let mut players_map: std::collections::HashMap<Pubkey, RoundPlayerDataWithAddress> = HashMap::new();

    // for ticket in round.tickets.iter() {
    //     let entry = players_map.entry(ticket.owner).or_insert(RoundPlayerDataWithAddress {
    //         player: ticket.owner,
    //         tickets_count: 0,
    //         has_bonus_ticket: false,
    //     });

    //     entry.tickets_count += 1;
    //     if ticket.is_bonus {
    //         entry.has_bonus_ticket = true;
    //     }
    // }

    let round_players: Vec<RoundPlayerDataWithAddress> = players_map.into_values().collect();

    Ok(RoundDataView {
        round_id: round.round_id,
        token_raffle: round.token_raffle,
        status: round.status.clone(),
        start_time: round.start_time,
        end_time: round.end_time,
        prize_amount: round.prize_amount,
        commission_balance: round.commission_balance,
        //cumulative_tickets: round.cumulative_tickets.clone(),
        winner_address: round.winner_address,
        winner_purchase_index: round.winner_purchase_index,
        winner_ticket_index: round.winner_ticket_index,
        round_players,
        prize_claimed: round.prize_claimed,
    })
}

pub fn get_current_raffle_round_id(
    ctx: Context<GetTockenRaffle>,
) -> Result<u32> {
    let sol_raffle = &ctx.accounts.sol_raffle;
    let now = Clock::get()?.unix_timestamp;

    let current_round_id = sol_raffle.current_round_id.unwrap_or(0);

    let should_increment = sol_raffle
        .current_round_end_time
        .map(|end_time| end_time <= now)
        .unwrap_or(false);

    if should_increment {
        Ok(current_round_id + 1)
    } else {
        Ok(current_round_id)
    }
}

pub fn get_raffle_round_count(ctx: Context<GetTockenRaffle>) -> Result<u32> {
    Ok(ctx.accounts.sol_raffle.total_rounds)
}
    
pub fn get_ticket_price_in_btc(ctx: Context<EmptyAccounts>) -> Result<u64> {
    Ok(TICKET_BTC_SATOSHIS)
}

pub fn get_ticket_price(ctx: Context<GetTicketPrice>) -> Result<u64> {
    let ticket_price = calculate_ticket_price_for_sol(
        &ctx.accounts.btc_price_feed,
        &ctx.accounts.sol_price_feed,
    )?;

    Ok(ticket_price)
}

#[derive(Accounts)]
pub struct GetRaffleState<'info> {
    #[account(
        seeds = [b"raffle_state"],
        bump
    )]
    pub raffle_state: Account<'info, RaffleState>,
}

#[derive(Accounts)]
pub struct GetTokenRaffle<'info> {
    #[account(
        seeds = [b"sol_raffle"],
        bump = sol_raffle.bump
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,
}

#[derive(Accounts)]
pub struct GetTicketPrice<'info> {
    /// CHECK: Switchboard BTC price feed
    pub btc_price_feed: AccountInfo<'info>,

    /// CHECK: Switchboard SOL price feed
    pub sol_price_feed: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct GetTockenRaffle<'info> {
    #[account(
        seeds = [b"sol_raffle"],
        bump = sol_raffle.bump
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,
}

#[derive(Accounts)]
#[instruction(round_id: u32)]
pub struct GetRoundAccounts<'info> {
    #[account(
        seeds = [b"sol_raffle"],
        bump = sol_raffle.bump
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,

    #[account(
        seeds = [
            b"round",
            sol_raffle.key().as_ref(),
            &round_id.to_le_bytes()
        ],
        bump
    )]
    pub round: Account<'info, Round>,
}

#[derive(Accounts)]
pub struct EmptyAccounts {}

// // View data structures
#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct RaffleStateView {
    pub authority: Pubkey,
    pub entrance_fee_percentage: u8,
    pub beneficiary: Pubkey,
    pub created_at: i64,
    pub vrf_request_counter: u8,
    pub test_ticket_price: Option<u64>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct TokenRaffleView {
    pub authority: Pubkey,
    pub token_mint: Pubkey,
    pub current_round_id: Option<u32>,
    pub current_round_status: RoundStatus,
    pub current_round_end_time: Option<i64>,
    pub total_rounds: u32,
    pub pending_rounds: Vec<u32>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct RoundPlayerDataWithAddress {
    pub player: Pubkey,
    pub tickets_count: u32,
    pub has_bonus_ticket: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct RoundResultView {
    pub winner_address: Option<Pubkey>,
    pub winner_ticket_index: Option<u32>,
    pub round_players: Vec<RoundPlayerDataWithAddress>,
    pub prize_amount: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct RoundDataView {
    pub round_id: u32,
    pub token_raffle: Pubkey,
    pub status: RoundStatus,
    pub start_time: i64,
    pub end_time: i64,
    pub prize_amount: u64,
    pub commission_balance: u64,
    pub winner_address: Option<Pubkey>,
    pub winner_purchase_index: Option<u32>,
    pub winner_ticket_index: Option<u32>,
    pub round_players: Vec<RoundPlayerDataWithAddress>,
    pub prize_claimed: bool,
}

// Error codes
#[error_code]
pub enum ErrorCode {
    #[msg("Round does not exist")]
    RoundDoesNotExist,
}
