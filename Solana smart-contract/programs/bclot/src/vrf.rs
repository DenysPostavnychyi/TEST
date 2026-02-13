use anchor_lang::prelude::*;
use orao_solana_vrf_cb::{
    cpi,
    program::OraoVrfCb,
    state::{
        client::{Callback, Client, RemainingAccount},
        network_state::NetworkState,
        request::RequestAccount,
    },
    RequestParams, CB_CLIENT_ACCOUNT_SEED, CB_CONFIG_ACCOUNT_SEED, CB_REQUEST_ACCOUNT_SEED,
};
use bytemuck::Pod;
use crate::{Round, TokenRaffle, RoundStatus, RaffleError};
use crate::change_round_status;
use crate::CLIENT_STATE_SEED;
use crate::ClientState;
use crate::RoundTickets;

pub fn initialize_client_state(ctx: Context<InitializeClientState>) -> Result<()> {
    let client_state = &mut ctx.accounts.client_state;
    
    client_state.bump = ctx.bumps.client_state;
    client_state.authority = ctx.accounts.authority.key();
    
    msg!("✅ Client State initialized");
    msg!("📍 Client State PDA: {}", client_state.key());
    msg!("📍 Bump: {}", client_state.bump);
    msg!("📍 Authority: {}", client_state.authority);
    
    Ok(())
}

pub fn consume_randomness(
    ctx: Context<ConsumeRandomness>,
    round_id: u32
) -> Result<()> {
    msg!("TEST consume_randomness");

    let randomness = ctx
        .accounts
        .request
        .fulfilled()
        .expect("must be fulfilled")
        .randomness;
    msg!("randomness: {:?}", randomness);

    let random_number = u64::from_le_bytes(randomness[0..8].try_into().unwrap());
    msg!("random_number: {}", random_number);

    let sol_raffle = &mut ctx.accounts.sol_raffle;
    msg!("Sol_raffle: {:?}", sol_raffle);
    let round = &mut ctx.accounts.round;
    msg!("Round: {:?}", round);

    let round_pubkey = round.key();
    let round_tickets_seeds = &[
        b"round_tickets",
        round_pubkey.as_ref(),
    ];
    
    let (round_tickets_pubkey, _bump) = 
        Pubkey::find_program_address(round_tickets_seeds, &crate::id());
    
    let round_tickets_info = ctx.remaining_accounts
        .iter()
        .find(|acc| acc.key == &round_tickets_pubkey)
        .ok_or(RaffleError::NotEnoughRemainingAccounts)?;

    pick_winner(sol_raffle, round, round_tickets_info, random_number)?;

    sol_raffle.pending_rounds.retain(|&id| id != round_id);
    
    Ok(())
}

fn pick_winner(
    sol_raffle: &mut Account<TokenRaffle>,
    round: &mut Account<Round>,
    round_tickets_info: &AccountInfo,
    random_number: u64,
) -> Result<()> {
    msg!("TEST pick_winner");

    if round.status == RoundStatus::Completed {
        msg!("⚠️ Round {} already completed, skipping pick_winner", round.round_id);
        return Ok(());
    }

    // Ensure the account has enough allocated space to safely read or modify its content
    let data = round_tickets_info.try_borrow_data()?;
    require!(
        data.len() >= 8 + std::mem::size_of::<RoundTickets>(),
        RaffleError::InvalidAccountSize
    );
    
    let round_tickets: &RoundTickets = bytemuck::from_bytes(&data[8..]); // Skip 8-byte discriminator
    
    let tickets = round_tickets.get_tickets();
    msg!("tickets_length: {}", tickets.len());
    
    let winner_ticket_index = (random_number % round.total_tickets as u64) as u32;
    msg!("Winner ticket index: {}", winner_ticket_index);

    let purchase_index = tickets.partition_point(|&c| c <= winner_ticket_index);

    if purchase_index >= tickets.len() {
        msg!("ERROR: winner_ticket_number {} not found", winner_ticket_index);
        return Err(RaffleError::TicketNotFound.into());
    }

    round.winner_purchase_index = Some(purchase_index as u32);
    round.winner_ticket_index = Some(winner_ticket_index);

    let res = change_round_status(sol_raffle, round, RoundStatus::Completed)?;
    msg!("change_round_status res: {:?}", res);

    let clock = Clock::get()?;

    emit!(WinnerPicked {
        token: sol_raffle.token_mint,
        round: round.key(),
        round_id: round.round_id,
        winner_purchase_index: purchase_index as u32,
        winner_ticket_index: winner_ticket_index,
        prize_amount: round.prize_amount,
        timestamp: clock.unix_timestamp,
    });

    msg!("TEST Winner picked");
    Ok(())
}

// Account validation structs
#[derive(Accounts)]
pub struct InitializeClientState<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + std::mem::size_of::<ClientState>(),
        seeds = [CLIENT_STATE_SEED],
        bump
    )]
    pub client_state: Account<'info, ClientState>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(round_id: u32)]
pub struct ConsumeRandomness<'info> {
    #[account(
        signer,
        seeds = [CB_CLIENT_ACCOUNT_SEED, crate::id().as_ref(), client.state.as_ref()],
        seeds::program = orao_solana_vrf_cb::id(),
        bump = client.bump
    )]
    pub client: Account<'info, Client>,

    #[account(
        mut,
        seeds = [CLIENT_STATE_SEED],
        bump = client_state.bump
    )]
    pub client_state: Account<'info, ClientState>,

    #[account(
        seeds = [CB_CONFIG_ACCOUNT_SEED],
        seeds::program = orao_solana_vrf_cb::id(),
        bump = network_state.bump,
    )]
    pub network_state: Account<'info, NetworkState>,

    #[account(
        seeds = [CB_REQUEST_ACCOUNT_SEED, client.key().as_ref(), request.seed()],
        seeds::program = orao_solana_vrf_cb::id(),
        bump = request.bump,
    )]
    pub request: Account<'info, RequestAccount>,

    #[account(
        mut,
        seeds = [b"sol_raffle"],
        bump = sol_raffle.bump
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,

    #[account(
        mut,
        seeds = [
            b"round",
            sol_raffle.key().as_ref(),
            &round_id.to_le_bytes()
        ],
        bump = round.bump
    )]
    pub round: Account<'info, Round>,
}

// Events
#[event]
pub struct WinnerPicked {
    pub token: Pubkey,
    pub round: Pubkey,
    pub round_id: u32,
    pub winner_purchase_index: u32,
    pub winner_ticket_index: u32,
    pub prize_amount: u64,
    pub timestamp: i64,
}