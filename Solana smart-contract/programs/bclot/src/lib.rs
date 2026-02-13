use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::invoke_signed,
    system_instruction,
};
use anchor_lang::system_program::{self, CreateAccount};
use anchor_lang::Discriminator;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::str::FromStr;
use switchboard_on_demand::on_demand::accounts::pull_feed::PullFeedAccountData;
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
use bytemuck::{Pod, Zeroable};
use std::cell::RefMut; 

declare_id!("Ah737jVNXFRoUMo8qyCGhBW4HyFz6MvKMVvEkgqm5o85");

pub mod admin;
pub mod vrf;

use admin::*;
use vrf::*;

// Constants
const LAMPORTS_PER_SOL: u64 = 1_000_000_000u64;
const TICKET_BTC_STR: &str = "0.00005";
const CLIENT_STATE_SEED: &[u8] = b"CLIENT_STATE";
pub const TICKET_BTC_SATOSHIS: u64 = 5_000; // 0.00005 BTC = 5,000 satoshi
const BTC_DECIMALS: u8 = 8;
const USD_DECIMALS: u8 = 6;
const SECONDS_IN_DAY: i64 = 86400;
const NY_OFFSET: i64 = 4 * 3600; // UTC-4
const ROUND_DURATION: i64 = 600; // 10 minutes

#[program]
pub mod raffle {
    use super::*;

    pub fn get_raffle_state(ctx: Context<GetRaffleState>) -> Result<RaffleStateView> {
        admin::get_raffle_state(ctx)
    }

    pub fn get_token_raffle(ctx: Context<GetTokenRaffle>) -> Result<TokenRaffleView> {
        admin::get_token_raffle(ctx)
    }

    pub fn get_ticket_price_in_btc(ctx: Context<EmptyAccounts>) -> Result<u64> {
        admin::get_ticket_price_in_btc(ctx)
    }

    pub fn get_ticket_price(ctx: Context<GetTicketPrice>) -> Result<u64> {
        admin::get_ticket_price(ctx)
    }

    pub fn get_raffle_round_result(ctx: Context<GetRoundAccounts>, round_id: u32) -> Result<RoundResultView> {
        admin::get_raffle_round_result(ctx, round_id)
    }

    pub fn get_raffle_round_data(ctx: Context<GetRoundAccounts>, round_id: u32) -> Result<RoundDataView> {
        admin::get_raffle_round_data(ctx, round_id)
    }

    pub fn get_current_raffle_round_id(ctx: Context<GetTockenRaffle>) -> Result<u32> {
        admin::get_current_raffle_round_id(ctx)
    }

    pub fn get_raffle_round_count(ctx: Context<GetTockenRaffle>) -> Result<u32> {
        admin::get_raffle_round_count(ctx)
    }

    //TEST
    pub fn get_cumulative_tickets(
        ctx: Context<GetCumulativeTickets>,
        round_id: u32,
    ) -> Result<Vec<u32>> {
        let round = &ctx.accounts.round;

        require!(round.initialized, RaffleError::RoundNotCreated);
        require!(round.round_id == round_id, RaffleError::RoundNotCreated);

        let round_tickets = ctx.accounts.round_tickets.load()?;

        msg!(
            "📖 Reading cumulative_tickets for round {} (len={})",
            round_id,
            round_tickets.len
        );

        Ok(round_tickets.get_tickets().to_vec())
    }
    //TEST

    pub fn initialize_raffle(
        ctx: Context<InitializeRaffle>,
        entrance_fee_percentage: u8,
        beneficiary: Pubkey,
    ) -> Result<()> {
        
        let raffle_state = &mut ctx.accounts.raffle_state;
        raffle_state.authority = ctx.accounts.authority.key();
        raffle_state.entrance_fee_percentage = entrance_fee_percentage;
        raffle_state.beneficiary = beneficiary;
        raffle_state.created_at = Clock::get()?.unix_timestamp;
        raffle_state.vrf_request_counter = 0;
        raffle_state.bump = ctx.bumps.raffle_state;
        raffle_state.test_ticket_price = None;

        msg!("RaffleState initialized successfully");
        msg!("Raffle State PDA: {}", raffle_state.key());

        Ok(())
    }

    pub fn initialize_sol_raffle(ctx: Context<InitializeSolRaffle>) -> Result<()> {
        let sol_raffle = &mut ctx.accounts.sol_raffle;
        
        sol_raffle.authority = ctx.accounts.authority.key();
        sol_raffle.total_rounds = 0;
        sol_raffle.bump = ctx.bumps.sol_raffle;

        msg!("TokenRaffle initialized successfully");
        msg!("Sol Raffle PDA: {}", sol_raffle.key());
        
        Ok(())
    }

    pub fn initialize_client_state(ctx: Context<InitializeClientState>) -> Result<()> {
        vrf::initialize_client_state(ctx)
    }
    
    pub fn fund_vrf_vault(
        ctx: Context<FundVrfVault>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, RaffleError::InvalidAmount);
        
        let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
            ctx.accounts.funder.key,
            ctx.accounts.vrf_fee_vault.key,
            amount,
        );
        
        anchor_lang::solana_program::program::invoke(
            &transfer_ix,
            &[
                ctx.accounts.funder.to_account_info(),
                ctx.accounts.vrf_fee_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        
        msg!("VRF vault funded with {} lamports", amount);
        Ok(())
    }

    /// Only authority
    pub fn withdraw_vrf_vault(
        ctx: Context<WithdrawVrfVault>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, RaffleError::InvalidAmount);
        
        let vault_balance = ctx.accounts.vrf_fee_vault.lamports();
        require!(
            vault_balance >= amount,
            RaffleError::InsufficientFunds
        );
        
        let vrf_vault_bump = ctx.bumps.vrf_fee_vault;
        
        **ctx.accounts.vrf_fee_vault.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.authority.to_account_info().try_borrow_mut_lamports()? += amount;
        
        msg!("Withdrawn {} lamports from VRF vault", amount);
        Ok(())
    }

    pub fn fund_rent_vault(
        ctx: Context<FundRentVault>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, RaffleError::InvalidAmount);
        
        let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
            ctx.accounts.funder.key,
            ctx.accounts.rent_vault.key,
            amount,
        );
        
        anchor_lang::solana_program::program::invoke(
            &transfer_ix,
            &[
                ctx.accounts.funder.to_account_info(),
                ctx.accounts.rent_vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        
        msg!("VRF vault funded with {} lamports", amount);
        Ok(())
    }

    /// Only authority
    pub fn withdraw_rent_vault(
        ctx: Context<WithdrawRentVault>,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, RaffleError::InvalidAmount);
        
        let vault_balance = ctx.accounts.rent_vault.lamports();
        require!(
            vault_balance >= amount,
            RaffleError::InsufficientFunds
        );
        
        let vrf_vault_bump = ctx.bumps.rent_vault;
        
        **ctx.accounts.rent_vault.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.authority.to_account_info().try_borrow_mut_lamports()? += amount;
        
        msg!("Withdrawn {} lamports from VRF vault", amount);
        Ok(())
    }

//TEST
pub fn initialize_round(
    ctx: Context<InitializeRound>,
    round_id: u32,
) -> Result<()> {
    let clock = Clock::get()?;
    let rent = Rent::get()?;
    let current_time = clock.unix_timestamp;
    let round_end_time = get_temporary_close_time(current_time);

    let sol_raffle = &mut ctx.accounts.sol_raffle;
    let sol_raffle_key = sol_raffle.key();
    
    // Bumps
    let vault_bump = ctx.bumps.rent_vault;
    let round_bump = ctx.bumps.round;
    let round_tickets_bump = ctx.bumps.round_tickets;
    
    let vault_seeds = &[b"rent_vault".as_ref(), &[vault_bump]];
    
    // ========== 1. Create Round account ==========
    let round_seeds = &[
        b"round".as_ref(),
        sol_raffle_key.as_ref(),
        &round_id.to_le_bytes(),
        &[round_bump],
    ];
    let round_space = 8 + Round::INIT_SPACE;
    let round_lamports = rent.minimum_balance(round_space);

    invoke_signed(
        &system_instruction::create_account(
            &ctx.accounts.rent_vault.key(),
            &ctx.accounts.round.key(),
            round_lamports,
            round_space as u64,
            ctx.program_id,
        ),
        &[
            ctx.accounts.rent_vault.to_account_info(),
            ctx.accounts.round.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
        &[vault_seeds, round_seeds],
    )?;
    
    // Initialize Round
    {
        let mut round_account_data = ctx.accounts.round.try_borrow_mut_data()?;
        
        let round_data = Round {
            initialized: true,
            token_raffle: sol_raffle_key,
            round_id,
            status: RoundStatus::Open,
            start_time: current_time,
            end_time: round_end_time,
            prize_amount: 0,
            commission_balance: 0,
            purchases_count: 0,
            total_tickets: 0,
            winner_ticket_index: None,
            winner_purchase_index: None,
            winner_address: None,
            prize_claimed: false,
            bump: round_bump,
        };
        
        let mut writer = &mut round_account_data[..];
        round_data.try_serialize(&mut writer)?;
    }

    // ========== 2. Create RoundTickets account ==========
    let round_key = ctx.accounts.round.key();
    let round_tickets_seeds = &[
        b"round_tickets".as_ref(),
        round_key.as_ref(),
        &[round_tickets_bump],
    ];
    let tickets_space = 8 + std::mem::size_of::<RoundTickets>();
    let tickets_lamports = rent.minimum_balance(tickets_space);
    
    invoke_signed(
        &system_instruction::create_account(
            &ctx.accounts.rent_vault.key(),
            &ctx.accounts.round_tickets.key(),
            tickets_lamports,
            tickets_space as u64,
            ctx.program_id,
        ),
        &[
            ctx.accounts.rent_vault.to_account_info(),
            ctx.accounts.round_tickets.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
        &[vault_seeds, round_tickets_seeds],
    )?;

    // Initialize RoundTickets
    {
        let mut tickets_data = ctx.accounts.round_tickets.try_borrow_mut_data()?;
        let discriminator = RoundTickets::DISCRIMINATOR;
        tickets_data[0..8].copy_from_slice(&discriminator);
        
        for byte in &mut tickets_data[8..] {
            *byte = 0;
        }
        
        tickets_data[8..40].copy_from_slice(round_key.as_ref());
        
        let bump_offset = 8 + 32 + (1024 * 4) + (1024 * 4) + 4;
        tickets_data[bump_offset] = round_tickets_bump;
    }

    msg!(
        "✅ Round {} and RoundTickets created (paid {} lamports total)", 
        round_id, 
        round_lamports + tickets_lamports
    );

    if let Some(prev_id) = sol_raffle.current_round_id {
        let prev_round_end = sol_raffle.current_round_end_time.unwrap();

        if current_time >= prev_round_end {
            
            if sol_raffle.current_round_status == RoundStatus::Open {
                if !sol_raffle.pending_rounds.contains(&prev_id) {
                    sol_raffle.pending_rounds.push(prev_id);
                    msg!("✅ Added round {} to pending (status: Open)", prev_id);
                }
            } else {
                msg!("⚠️ Round {} already Completed, not adding to pending", prev_id);
            }
        }
    }

    sol_raffle.current_round_id = Some(round_id);
    sol_raffle.total_rounds = sol_raffle
        .total_rounds
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    sol_raffle.current_round_status = RoundStatus::Open;
    sol_raffle.current_round_end_time = Some(round_end_time);

    Ok(())
}
//TEST

pub fn buy_tickets_sol(
    ctx: Context<BuyTicketsSol>,
    round_id: u32,
    purchase_index: u32,
    count: u32,
    max_cost: u64,
) -> Result<()> {
    msg!("Tickets count: {}", count);
    require!(count > 0, RaffleError::InvalidTicketCount);

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;
    let sol_raffle = &mut ctx.accounts.sol_raffle;
    
    let round_exists = ctx.accounts.round.owner == ctx.program_id 
        && ctx.accounts.round.data_len() > 0;
    
    if !round_exists {
        msg!("🆕 Round {} doesn't exist, creating...", round_id);
        
        let rent = Rent::get()?;
        let sol_raffle_key = sol_raffle.key();
        
        let vault_bump = ctx.bumps.rent_vault;
        let round_bump = ctx.bumps.round;
        let round_tickets_bump = ctx.bumps.round_tickets;
        
        let vault_seeds = &[b"rent_vault".as_ref(), &[vault_bump]];
        
        // ========== Create Round account ==========
        let round_seeds = &[
            b"round".as_ref(),
            sol_raffle_key.as_ref(),
            &round_id.to_le_bytes(),
            &[round_bump],
        ];
        let round_space = 8 + Round::INIT_SPACE;
        let round_lamports = rent.minimum_balance(round_space);

        invoke_signed(
            &system_instruction::create_account(
                &ctx.accounts.rent_vault.key(),
                &ctx.accounts.round.key(),
                round_lamports,
                round_space as u64,
                ctx.program_id,
            ),
            &[
                ctx.accounts.rent_vault.to_account_info(),
                ctx.accounts.round.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[vault_seeds, round_seeds],
        )?;
        
        // Initialize Round
        {
            let round_end_time = get_temporary_close_time(current_time);
            let mut round_account_data = ctx.accounts.round.try_borrow_mut_data()?;
            
            let round_data = Round {
                initialized: true,
                token_raffle: sol_raffle_key,
                round_id,
                status: RoundStatus::Open,
                start_time: current_time,
                end_time: round_end_time,
                prize_amount: 0,
                commission_balance: 0,
                purchases_count: 0,
                total_tickets: 0,
                winner_ticket_index: None,
                winner_purchase_index: None,
                winner_address: None,
                prize_claimed: false,
                bump: round_bump,
            };
            
            let mut writer = &mut round_account_data[..];
            round_data.try_serialize(&mut writer)?;
        }

        // ========== Create RoundTickets account ==========
        let round_key = ctx.accounts.round.key();
        let round_tickets_seeds = &[
            b"round_tickets".as_ref(),
            round_key.as_ref(),
            &[round_tickets_bump],
        ];
        let tickets_space = 8 + std::mem::size_of::<RoundTickets>();
        let tickets_lamports = rent.minimum_balance(tickets_space);
        
        invoke_signed(
            &system_instruction::create_account(
                &ctx.accounts.rent_vault.key(),
                &ctx.accounts.round_tickets.key(),
                tickets_lamports,
                tickets_space as u64,
                ctx.program_id,
            ),
            &[
                ctx.accounts.rent_vault.to_account_info(),
                ctx.accounts.round_tickets.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[vault_seeds, round_tickets_seeds],
        )?;

        // Initialize RoundTickets
        {
            let mut tickets_data = ctx.accounts.round_tickets.try_borrow_mut_data()?;
        
            let discriminator = RoundTickets::DISCRIMINATOR;
            tickets_data[0..8].copy_from_slice(&discriminator);
            
            for byte in &mut tickets_data[8..] {
                *byte = 0;
            }
            
            tickets_data[8..40].copy_from_slice(round_key.as_ref());
            
            let bump_offset = 8 + 32 + (1024 * 4) + (1024 * 4) + 4;
            tickets_data[bump_offset] = round_tickets_bump;
        }

        // Update sol_raffle state
        if let Some(prev_id) = sol_raffle.current_round_id {
            let prev_round_end = sol_raffle.current_round_end_time.unwrap();

            if current_time >= prev_round_end {
                if sol_raffle.current_round_status == RoundStatus::Open {
                    if !sol_raffle.pending_rounds.contains(&prev_id) {
                        sol_raffle.pending_rounds.push(prev_id);
                        msg!("✅ Added round {} to pending", prev_id);
                    }
                }
            }
        }

        sol_raffle.current_round_id = Some(round_id);
        sol_raffle.total_rounds = sol_raffle
            .total_rounds
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        sol_raffle.current_round_status = RoundStatus::Open;
        sol_raffle.current_round_end_time = Some(get_temporary_close_time(current_time));
        
        msg!("✅ Round {} created", round_id);
    }
    
    // Load and deserialize Round
    let round_data = ctx.accounts.round.try_borrow_data()?;
    let mut round_reader = &round_data[..];
    let mut round = Round::try_deserialize(&mut round_reader)?;
    drop(round_data); // Release the borrow immediately

    let current_round_id = sol_raffle.current_round_id.unwrap_or(0);
    let current_round_end_time = sol_raffle.current_round_end_time.unwrap_or(i64::MAX);

    let is_current_round = round_id == current_round_id && current_time < current_round_end_time;
    let is_next_round = round_id == current_round_id + 1 && current_time >= current_round_end_time;

    require!(
        is_current_round || is_next_round,
        RaffleError::RoundNotAvailable
    );

    require!(round.initialized, RaffleError::RoundNotInitialized);

    // Verify round_tickets connection
    {
        let tickets_data = ctx.accounts.round_tickets.try_borrow_data()?;
        let tickets_round_key = Pubkey::try_from(&tickets_data[8..40])
            .map_err(|_| RaffleError::RoundTicketsNotInitialized)?;
        require!(
            tickets_round_key == ctx.accounts.round.key(),
            RaffleError::RoundTicketsNotInitialized
        );
    }

    let round_tickets_purchase = &mut ctx.accounts.round_tickets_purchase;
    let round_tickets_purchase_bump = ctx.bumps.round_tickets_purchase;

    initialize_round_tickets_purchase(
        round_tickets_purchase,
        ctx.accounts.round.key(),
        round.purchases_count,
        ctx.accounts.player.key(),
        count,
        round_tickets_purchase_bump
    )?;

    let raffle_state = &ctx.accounts.raffle_state;
    let ticket_price: u64 = if let Some(test_price) = raffle_state.test_ticket_price {
        msg!("Using test ticket price: {} lamports", test_price);
        test_price
    } else {
        msg!("Calculating ticket price from oracle feeds");
        calculate_ticket_price_for_sol(
            &ctx.accounts.btc_price_feed,
            &ctx.accounts.sol_price_feed,
        )?
    };
    require!(ticket_price > 0, RaffleError::InvalidTicketPrice);

    let cost = ticket_price
        .checked_mul(count as u64)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    require!(cost <= max_cost, RaffleError::InsufficientSlippage);

    require!(
        ctx.accounts.player.lamports() >= cost,
        RaffleError::InsufficientFunds
    );

    let commission_amount = cost
        .checked_mul(raffle_state.entrance_fee_percentage as u64)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_div(100)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    round.commission_balance = round
        .commission_balance
        .checked_add(commission_amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let prize_amount = cost
        .checked_sub(commission_amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    round.prize_amount = round
        .prize_amount
        .checked_add(prize_amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let is_first_buyer = round.total_tickets == 0;

    let mut new_total = round
        .total_tickets
        .checked_add(count)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        ),
        cost,
    )?;

    if is_first_buyer {
        new_total = new_total + 1;

        round_tickets_purchase.tickets_count = round_tickets_purchase
            .tickets_count
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        emit!(FirstTicketBonusAwarded {
            token: sol_raffle.token_mint,
            round_id: round.round_id,
            buyer: ctx.accounts.player.key(),
            timestamp: current_time,
            round_start_time: round.start_time,
            round_end_time: round.end_time,
        });
    }

    // Update round_tickets using zero-copy mutation
    {
        let mut tickets_data = ctx.accounts.round_tickets.try_borrow_mut_data()?;
        
        // Read current len (offset: 8 + 32 + 4096 + 4096 = 8232)
        let len_offset = 8 + 32 + (1024 * 4) + (1024 * 4);
        let len = u32::from_le_bytes([
            tickets_data[len_offset],
            tickets_data[len_offset + 1],
            tickets_data[len_offset + 2],
            tickets_data[len_offset + 3],
        ]) as usize;
        
        // Write new cumulative value
        let cumulative_bytes = new_total.to_le_bytes();
        if len < 1024 {
            // Write to cumulative_tickets_1 (offset: 8 + 32)
            let ticket_offset = 8 + 32 + (len * 4);
            tickets_data[ticket_offset..ticket_offset + 4].copy_from_slice(&cumulative_bytes);
        } else {
            // Write to cumulative_tickets_2 (offset: 8 + 32 + 4096)
            let ticket_offset = 8 + 32 + (1024 * 4) + ((len - 1024) * 4);
            tickets_data[ticket_offset..ticket_offset + 4].copy_from_slice(&cumulative_bytes);
        }
        
        // Update len
        let new_len = (len + 1) as u32;
        tickets_data[len_offset..len_offset + 4].copy_from_slice(&new_len.to_le_bytes());
    }

    round.total_tickets = new_total;
    round.purchases_count += 1;

    {
        let mut final_round_data = ctx.accounts.round.try_borrow_mut_data()?;
        let mut writer = &mut final_round_data[..];
        round.try_serialize(&mut writer)?;
    }

    emit!(TicketPurchased {
        token: sol_raffle.token_mint,
        round_id: round.round_id,
        buyer: ctx.accounts.player.key(),
        count,
        total_amount: cost,
        prize_amount,
        commission_amount,
        timestamp: current_time,
    });

    Ok(())
}

    pub fn request_randomness<'info>(
        ctx: Context<'_, '_, '_, 'info, RequestRandomness<'info>>,
        seeds: Vec<[u8; 32]>,
    ) -> Result<()> {

        require_eq!(
            ctx.remaining_accounts.len(),
            seeds.len(),
            RaffleError::SeedMismatch
        );

        let sol_raffle = &mut ctx.accounts.sol_raffle;

        let round_id = determine_round_to_process(sol_raffle)?;
        msg!("round_id for winner pick: {}", round_id);

        //Checking for available request_accounts to use
        let raffle_state = &mut ctx.accounts.raffle_state;
        let idx = raffle_state.vrf_request_counter as usize;
        msg!("idx: {}", idx);
        msg!("seeds.len(): {}", seeds.len());
        require!(idx < seeds.len(), RaffleError::AllRequestsCompleted);

        let cpi_program = ctx.accounts.vrf.to_account_info();

        let vrf_fee_vault_bump = ctx.bumps.vrf_fee_vault;
        
        // Signature for both PDA: client_state and vrf_fee_vault
        let signers_seeds: &[&[&[u8]]] = &[
            &[CLIENT_STATE_SEED, &[ctx.accounts.client_state.bump]],
            &[b"vrf_fee_vault", &[vrf_fee_vault_bump]],
        ];

        let request_account = ctx
            .remaining_accounts
            .get(idx)
            .ok_or(RaffleError::NotEnoughRemainingAccounts)?;

        let seed = seeds
            .get(idx)
            .ok_or(RaffleError::NotEnoughSeeds)?;

        msg!("Preparing VRF request seed={:?} request_account={}", seed, request_account.key);

        let sol_raffle_key = sol_raffle.key();
        
        let round_seeds: &[&[u8]] = &[
            b"round",
            sol_raffle_key.as_ref(),
            &round_id.to_le_bytes(),
        ];

        let (round_pubkey, round_bump) =
            Pubkey::find_program_address(round_seeds, &crate::id());

        msg!("Computed round PDA = {} (round_id={})", round_pubkey, round_id);

        // Prepare callback instruction data
        let callback_ix = crate::instruction::ConsumeRandomness { round_id };

        let mut callback = Callback::from_instruction_data(&callback_ix);
        
        callback = callback.with_remaining_account(RemainingAccount::writable(
            sol_raffle_key,
            vec![
                b"sol_raffle".to_vec(),
                vec![sol_raffle.bump],
            ],
        ));
        
        callback = callback.with_remaining_account(RemainingAccount::writable(
            round_pubkey,
            vec![
                b"round".to_vec(),
                sol_raffle_key.as_ref().to_vec(),
                round_id.to_le_bytes().to_vec(), 
                vec![round_bump],
            ],
        ));

        let round_tickets_seeds: &[&[u8]] = &[
            b"round_tickets",
            round_pubkey.as_ref(),
        ];

        let (round_tickets_pubkey, round_tickets_bump) =
            Pubkey::find_program_address(round_tickets_seeds, &crate::id());

        msg!("Computed round_tickets PDA = {}", round_tickets_pubkey);

        callback = callback.with_remaining_account(RemainingAccount::writable(
            round_tickets_pubkey,
            vec![
                b"round_tickets".to_vec(),
                round_pubkey.as_ref().to_vec(),
                vec![round_tickets_bump],
            ],
        ));

        let mut cpi_accounts = cpi::accounts::Request {
            payer: ctx.accounts.vrf_fee_vault.to_account_info(),
            state: ctx.accounts.client_state.to_account_info(),
            client: ctx.accounts.client.to_account_info(),
            network_state: ctx.accounts.network_state.to_account_info(),
            treasury: ctx.accounts.treasury.to_account_info(),
            request: request_account.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
        };

        // Both accounts must be signer
        cpi_accounts.state.is_signer = true;
        cpi_accounts.payer.is_signer = true;

        let cpi_ctx = CpiContext::new(cpi_program.clone(), cpi_accounts).with_signer(signers_seeds);

        cpi::request(
            cpi_ctx,
            RequestParams::new(*seed).with_callback(Some(callback))
        )?;

        msg!("Submitted VRF request -> request_account={} round={}", request_account.key, round_pubkey);

        raffle_state.vrf_request_counter += 1;
        if seeds.len() == raffle_state.vrf_request_counter as usize {
            raffle_state.vrf_request_counter = 0;

            msg!("AllRequestsCompleted");
            emit!(AllRequestsCompleted {});
        }

        msg!("request_randomness done");
        Ok(())
    }

    pub fn consume_randomness(
        ctx: Context<ConsumeRandomness>,
        round_id: u32
    ) -> Result<()> {
        vrf::consume_randomness(ctx, round_id)
    }

    pub fn claim_prize_sol(ctx: Context<ClaimPrizeSol>, round_id: u32) -> Result<()> {

        let sol_raffle = &mut ctx.accounts.sol_raffle;
        let round = &mut ctx.accounts.round;
        let vault = &mut ctx.accounts.vault;
        
        require!(
            round.status == RoundStatus::Completed,
            RaffleError::RoundNotCompleted
        );
        require!(
            round.winner_address == Some(ctx.accounts.winner.key()),
            RaffleError::NotTheWinner
        );
        require!(!round.prize_claimed, RaffleError::PrizeAlreadyClaimed);

        let prize_amount = round.prize_amount;
        let commission_amount = round.commission_balance;
        let total_required = prize_amount
            .checked_add(commission_amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        msg!("prize_amount: {}", prize_amount);
        msg!("commission_amount: {}", commission_amount);
        msg!("total_required: {}", total_required);

        let vault_balance = **vault.lamports.borrow();
        require!(
            vault_balance >= total_required,
            RaffleError::InsufficientVaultBalance
        );
        
        let winner_balance_before = **ctx.accounts.winner.lamports.borrow();
        let beneficiary_balance_before = **ctx.accounts.beneficiary.lamports.borrow();
        msg!(
            "Winner balance before: {}",
            winner_balance_before
        );
        msg!(
            "Beneficiary balance before: {}",
            beneficiary_balance_before
        );

        // Transfer prize to winner
        **ctx.accounts.vault.try_borrow_mut_lamports()? -= prize_amount;
        **ctx.accounts.winner.try_borrow_mut_lamports()? += prize_amount;
        
        msg!("✓ Transferred {} lamports to winner", prize_amount);
        
        // Transfer commission to beneficiary
        **ctx.accounts.vault.try_borrow_mut_lamports()? -= commission_amount;
        **ctx.accounts.beneficiary.try_borrow_mut_lamports()? += commission_amount;
        
        msg!("✓ Transferred {} lamports to beneficiary", commission_amount);
        
        let winner_balance_after = **ctx.accounts.winner.lamports.borrow();
        let beneficiary_balance_after = **ctx.accounts.beneficiary.lamports.borrow();
        
        msg!("=== Transfer Completed ===");
        msg!(
            "Winner balance after: {} lamports (delta: +{})",
            winner_balance_after,
            prize_amount
        );
        msg!(
            "Beneficiary balance after: {} lamports (delta: +{})",
            beneficiary_balance_after,
            commission_amount
        );

        round.prize_claimed = true;
        emit!(PrizeClaimed {
            round_id: round.round_id,
            winner: ctx.accounts.winner.key(),
        });

        Ok(())
    }

    pub fn test_calculate_price(ctx: Context<TestCalculatePrice>) -> Result<()> {
        let price = calculate_ticket_price_for_sol(
            &ctx.accounts.btc_price_feed,
            &ctx.accounts.sol_price_feed,
        )?;

        msg!("✅ Final ticket price in lamports: {}", price);
        msg!(
            "✅ Final ticket price in SOL: {}",
            price as f64 / 1_000_000_000.0
        );

        Ok(())
    }

    //TEST
    pub fn set_test_ticket_price(
        ctx: Context<SetTestTicketPrice>,
        price: Option<u64>,
    ) -> Result<()> {
        let raffle_state = &mut ctx.accounts.raffle_state;
        raffle_state.test_ticket_price = price;
        
        if let Some(p) = price {
            msg!("Test ticket price set to: {} lamports", p);
        } else {
            msg!("Test ticket price disabled - using oracle calculation");
        }
        
        Ok(())
    }
    //TEST

    pub fn set_winner_address(
        ctx: Context<SetWinnerAddress>,
        round_id: u32,
        purchase_index: u32,
    ) -> Result<()> {
        let round = &mut ctx.accounts.round;
        let round_tickets_purchase = &ctx.accounts.round_tickets_purchase;

        require!(
            round.status == RoundStatus::Completed,
            RaffleError::RoundNotCompleted
        );

        require!(
            round.winner_address.is_none(),
            RaffleError::WinnerAlreadySet
        );

        round.winner_address = Some(round_tickets_purchase.player);

        msg!(
            "✅ Winner address set for round {}: {}",
            round_id,
            round_tickets_purchase.player
        );

        Ok(())
    }
}

fn determine_round_to_process(sol_raffle: &Account<TokenRaffle>) -> Result<u32> {
    if !sol_raffle.pending_rounds.is_empty() {
        let round_id = sol_raffle.pending_rounds[0];
        msg!("✅ Taking round {} from pending_rounds", round_id);
        return Ok(round_id);
    }
    
    let clock = Clock::get()?;
    let current_id = sol_raffle
        .current_round_id
        .ok_or(RaffleError::RoundNotCreated)?;
    
    msg!("Found current_round_id: {}", current_id);
    
    let end_time = sol_raffle
        .current_round_end_time
        .ok_or(RaffleError::RoundNotCreated)?;
    
    let status = &sol_raffle.current_round_status;
    
    msg!("Current round end time: {}", end_time);
    msg!("Current time: {}", clock.unix_timestamp);
    msg!("Current round status: {:?}", status);
    
    require!(*status == RoundStatus::Open, RaffleError::RoundNotOpen);
    require!(clock.unix_timestamp >= end_time, RaffleError::RoundNotEndedYet);
    
    Ok(current_id)
}

fn initialize_round_tickets_purchase(
    round_tickets_purchase: &mut Account<RoundTicketsPurchase>,
    round: Pubkey,
    purchases_count: u32,
    player: Pubkey,
    tickets_count: u32,
    round_tickets_purchase_bump: u8,
) -> Result<()> {
    round_tickets_purchase.round = round;
    round_tickets_purchase.player = player;
    round_tickets_purchase.purchase_index = purchases_count;
    round_tickets_purchase.tickets_count = tickets_count;
    round_tickets_purchase.bump = round_tickets_purchase_bump;
    msg!(" Round tickets purchase initialized");
    Ok(())
}

fn get_temporary_close_time(current_timestamp: i64) -> i64 {
    let round_active_time = current_timestamp % ROUND_DURATION;
    current_timestamp - round_active_time + ROUND_DURATION
}

pub fn change_round_status(
    sol_raffle: &mut Account<TokenRaffle>,
    round: &mut Account<Round>,
    new_status: RoundStatus,
) -> Result<()> {

    let old_status = round.status.clone();

    require!(old_status != RoundStatus::Completed, RaffleError::CannotChangeCompletedRound);

    round.status = new_status.clone();

    if sol_raffle.current_round_id == Some(round.round_id) {
        sol_raffle.current_round_status = new_status.clone();
        msg!("✅ Updated current_round_status to {:?}", new_status);
    } else {
        msg!("⚠️ Not updating current_round_status (processing old round {} while current is {:?})", 
            round.round_id, 
            sol_raffle.current_round_id
        );
    }

    emit!(StatusChanged {
        round_id: round.round_id,
        old_status,
        new_status,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

fn calculate_ticket_price_for_sol(
    btc_price_feed: &AccountInfo,
    sol_price_feed: &AccountInfo,
) -> Result<u64> {
    msg!("--- calculate_ticket_price_for_sol START ---");

    let clock = Clock::get()?;
    msg!("Current slot: {}", clock.slot);

    // ===== SOL feed =====
    msg!("Parsing SOL price feed...");
    let sol_data = sol_price_feed.data.borrow();

    let sol_feed = PullFeedAccountData::parse(sol_data).map_err(|e| {
        msg!("SOL Switchboard parse failed: {:?}", e);
        RaffleError::InvalidFeedAccount
    })?;

    let sol_price = sol_feed
        .get_value(clock.slot, 1500, 3, false)
        .map_err(|e| {
            msg!("SOL Switchboard get_value failed: {:?}", e);
            RaffleError::OracleError
        })?;

    msg!("SOL Price (Decimal): {}", sol_price);

    // ===== BTC feed =====
    msg!("Parsing BTC price feed...");
    let btc_data = btc_price_feed.data.borrow();

    let btc_feed = PullFeedAccountData::parse(btc_data).map_err(|e| {
        msg!("BTC Switchboard parse failed: {:?}", e);
        RaffleError::InvalidFeedAccount
    })?;

    let btc_price = btc_feed
        .get_value(clock.slot, 1500, 1, false)
        .map_err(|e| {
            msg!("BTC Switchboard get_value failed: {:?}", e);
            RaffleError::OracleError
        })?;

    msg!("BTC Price (Decimal): {}", btc_price);

    // ===== Ticket price calculation =====
    let ticket_price_btc =
        Decimal::from_str(TICKET_BTC_STR).map_err(|_| ProgramError::ArithmeticOverflow)?;

    msg!("Ticket price in BTC: {}", ticket_price_btc);

    let ticket_price_usd = btc_price
        .checked_mul(ticket_price_btc)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    msg!("Ticket price in USD: {}", ticket_price_usd);

    let ticket_price_sol = ticket_price_usd
        .checked_div(sol_price)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    msg!("Ticket price in SOL: {}", ticket_price_sol);

    let lamports_decimal = ticket_price_sol
        .checked_mul(Decimal::from(LAMPORTS_PER_SOL))
        .ok_or(ProgramError::ArithmeticOverflow)?;

    msg!("Lamports (Decimal): {}", lamports_decimal);

    let lamports_u128 = lamports_decimal
        .round()
        .to_u128()
        .ok_or(ProgramError::ArithmeticOverflow)?;

    msg!("Lamports (rounded, u128): {}", lamports_u128);

    let lamports = u64::try_from(lamports_u128)
        .map_err(|_| ProgramError::ArithmeticOverflow)?;

    msg!("Final ticket price (lamports): {}", lamports);
    msg!("--- calculate_ticket_price_for_sol END ---");

    Ok(lamports)
}

fn transfer<'a>(
    system_program: AccountInfo<'a>,
    from: AccountInfo<'a>,
    to: AccountInfo<'a>,
    amount: u64,
    seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    let amount_needed = amount;
    if amount_needed > from.lamports() {
        msg!(
            "Need {} lamports, but only have {}",
            amount_needed,
            from.lamports()
        );
        return Err(RaffleError::NotEnoughFundsToPlay.into());
    }

    let transfer_accounts = anchor_lang::system_program::Transfer {
        from: from.to_account_info(),
        to: to.to_account_info(),
    };

    let transfer_ctx = match seeds {
        Some(seeds) => CpiContext::new_with_signer(system_program, transfer_accounts, seeds),
        None => CpiContext::new(system_program, transfer_accounts),
    };

    anchor_lang::system_program::transfer(transfer_ctx, amount)
}

// Account validation structs

//TEST
#[derive(Accounts)]
#[instruction(round_id: u32)]
pub struct GetCumulativeTickets<'info> {
    #[account(
        seeds = [
            b"round",
            sol_raffle.key().as_ref(),
            &round_id.to_le_bytes()
        ],
        bump = round.bump
    )]
    pub round: Account<'info, Round>,

    #[account(
        seeds = [b"sol_raffle"],
        bump = sol_raffle.bump
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,

    #[account(
        seeds = [
            b"round_tickets",
            round.key().as_ref()
        ],
        bump
    )]
    pub round_tickets: AccountLoader<'info, RoundTickets>,
}
//TEST

#[derive(Accounts)]
pub struct FundRentVault<'info> {
    
    /// PDA for storing funds fo rent
    #[account(
        mut,
        seeds = [b"rent_vault"],
        bump
    )]
    pub rent_vault: SystemAccount<'info>,

    #[account(mut)]
    pub funder: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawRentVault<'info> {
    #[account(
        mut,
        seeds = [b"raffle_state"],
        bump = raffle_state.bump,
        has_one = authority @ RaffleError::Unauthorized
    )]
    pub raffle_state: Account<'info, RaffleState>,
    
    #[account(
        mut,
        seeds = [b"rent_vault"],
        bump
    )]
    pub rent_vault: SystemAccount<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

//TEST
#[derive(Accounts)]
#[instruction(round_id: u32)]
pub struct InitializeRound<'info> {
    #[account(
        mut,
        seeds = [b"sol_raffle"],
        bump = sol_raffle.bump,
        constraint = sol_raffle.authority == authority.key() @ RaffleError::Unauthorized
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,
    
    /// CHECK: Created through CPI
    #[account(
        mut,
        seeds = [b"round", sol_raffle.key().as_ref(), &round_id.to_le_bytes()],
        bump
    )]
    pub round: UncheckedAccount<'info>,

    /// CHECK: Created through CPI
    #[account(
        mut,
        seeds = [b"round_tickets", round.key().as_ref()],
        bump
    )]
    pub round_tickets: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"rent_vault"],
        bump
    )]
    pub rent_vault: SystemAccount<'info>,
    
    #[account(mut)]
    pub authority: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}
//TEST

#[derive(Accounts)]
pub struct InitializeRaffle<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + RaffleState::INIT_SPACE,
        seeds = [b"raffle_state"],
        bump
    )]
    pub raffle_state: Account<'info, RaffleState>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,

    #[account(
        init,
        payer = authority,
        space = 0,
        seeds = [b"sol_vault"],
        bump
    )]
    /// CHECK: vault for storing SOL
    pub vault: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct InitializeSolRaffle<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + TokenRaffle::INIT_SPACE,
        seeds = [b"sol_raffle"],
        bump
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FundVrfVault<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    
    /// PDA for storing funds for VRF
    #[account(
        mut,
        seeds = [b"vrf_fee_vault"],
        bump
    )]
    pub vrf_fee_vault: SystemAccount<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawVrfVault<'info> {
    #[account(
        mut,
        seeds = [b"raffle_state"],
        bump = raffle_state.bump,
        has_one = authority @ RaffleError::Unauthorized
    )]
    pub raffle_state: Account<'info, RaffleState>,
    
    #[account(mut)]
    pub authority: Signer<'info>,
    
    #[account(
        mut,
        seeds = [b"vrf_fee_vault"],
        bump
    )]
    pub vrf_fee_vault: SystemAccount<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(round_id: u32, purchase_index: u32)]
pub struct BuyTicketsSol<'info> {
    #[account(
        seeds = [b"raffle_state"],
        bump = raffle_state.bump
    )]
    pub raffle_state: Account<'info, RaffleState>,

    #[account(
        mut,
        seeds = [b"sol_raffle"],
        bump = sol_raffle.bump
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,

    /// CHECK: May not exist - will be created via CPI if needed
    #[account(
        mut,
        seeds = [
            b"round",
            sol_raffle.key().as_ref(),
            &round_id.to_le_bytes()
        ],
        bump
    )]
    pub round: UncheckedAccount<'info>,

    /// CHECK: May not exist - will be created via CPI if needed
    #[account(
        mut,
        seeds = [
            b"round_tickets",
            round.key().as_ref()
        ],
        bump
    )]
    pub round_tickets: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"rent_vault"],
        bump
    )]
    pub rent_vault: SystemAccount<'info>,

    #[account(
        init,
        payer = player,
        space = 8 + RoundTicketsPurchase::INIT_SPACE,
        seeds = [
            b"round_tickets_purchase",
            round.key().as_ref(),
            &purchase_index.to_le_bytes()
        ],
        bump
    )]
    pub round_tickets_purchase: Account<'info, RoundTicketsPurchase>,

    #[account(mut)]
    pub player: Signer<'info>,

    #[account(
        mut,
        seeds = [b"sol_vault"],
        bump
    )]
    /// CHECK: vault for storing SOL
    pub vault: AccountInfo<'info>,

    /// CHECK: BTC price feed account
    pub btc_price_feed: AccountInfo<'info>,

    /// CHECK: SOL price feed account
    pub sol_price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(round_id: u32)]
pub struct ClaimPrizeSol<'info> {
    #[account(
        mut,
        seeds = [b"raffle_state"],
        bump = raffle_state.bump
    )]
    pub raffle_state: Account<'info, RaffleState>,

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

    #[account(
        mut,
        close = rent_vault,
        seeds = [
            b"round_tickets",
            round.key().as_ref()
        ],
        bump
    )]
    pub round_tickets: AccountLoader<'info, RoundTickets>,

    #[account(mut)]
    pub winner: Signer<'info>,

    #[account(
        mut,
        seeds = [b"sol_vault"],
        bump
    )]
    /// CHECK: vault for storing SOL
    pub vault: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [b"rent_vault"],
        bump
    )]
    pub rent_vault: SystemAccount<'info>,

    #[account(
        mut,
        constraint = beneficiary.key() == raffle_state.beneficiary @ RaffleError::InvalidBeneficiary
    )]
    /// CHECK: key validated via constraint
    pub beneficiary: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct TestCalculatePrice<'info> {
    /// CHECK: Switchboard BTC price feed
    pub btc_price_feed: AccountInfo<'info>,
    /// CHECK: Switchboard SOL price feed
    pub sol_price_feed: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct RequestRandomness<'info> {
   
    #[account(
        mut,
        seeds = [b"raffle_state"],
        bump = raffle_state.bump
    )]
    pub raffle_state: Account<'info, RaffleState>,

    #[account(
        mut,
        seeds = [b"sol_raffle"],
        bump = sol_raffle.bump
    )]
    pub sol_raffle: Account<'info, TokenRaffle>,

    /// PDA for VRF payment
    #[account(
        mut,
        seeds = [b"vrf_fee_vault"],
        bump
    )]
    pub vrf_fee_vault: SystemAccount<'info>,

    pub vrf: Program<'info, OraoVrfCb>,

    /// State of a registered client.
    #[account(
        mut,
        seeds = [CLIENT_STATE_SEED],
        bump = client_state.bump
    )]
    pub client_state: Account<'info, ClientState>,

    /// Registered client PDA.
    #[account(
        mut,
        seeds = [CB_CLIENT_ACCOUNT_SEED, crate::id().as_ref(), client_state.key().as_ref()],
        seeds::program = orao_solana_vrf_cb::id(),
        bump = client.bump,
    )]
    pub client: Account<'info, Client>,

    #[account(
        mut,
        seeds = [CB_CONFIG_ACCOUNT_SEED],
        seeds::program = orao_solana_vrf_cb::id(),
        bump = network_state.bump,
    )]
    pub network_state: Account<'info, NetworkState>,

    /// CHECK: Treasury ORAO VRF
    #[account(mut)]
    pub treasury: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

//TEST
#[derive(Accounts)]
pub struct SetTestTicketPrice<'info> {
    #[account(
        mut,
        seeds = [b"raffle_state"],
        bump = raffle_state.bump
    )]
    pub raffle_state: Account<'info, RaffleState>,
    
    pub authority: Signer<'info>,
}
//TEST

#[derive(Accounts)]
#[instruction(round_id: u32, purchase_index: u32)]
pub struct SetWinnerAddress<'info> {
    #[account(
        mut,
        seeds = [b"raffle_state"],
        bump = raffle_state.bump,
        has_one = authority @ RaffleError::Unauthorized
    )]
    pub raffle_state: Account<'info, RaffleState>,

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

    #[account(
        seeds = [
            b"round_tickets_purchase",
            round.key().as_ref(),
            &purchase_index.to_le_bytes()
        ],
        bump = round_tickets_purchase.bump,
        constraint = round.winner_purchase_index == Some(purchase_index) @ RaffleError::InvalidPurchaseIndex
    )]
    pub round_tickets_purchase: Account<'info, RoundTicketsPurchase>,

    pub authority: Signer<'info>,
}

// Account data structs
#[account]
#[derive(InitSpace)]
pub struct RaffleState {
    pub authority: Pubkey,
    pub entrance_fee_percentage: u8,
    pub beneficiary: Pubkey,
    pub created_at: i64,
    pub vrf_request_counter: u8,
    pub bump: u8,
    pub test_ticket_price: Option<u64>,
}

#[account]
#[derive(InitSpace)]
pub struct ClientState {
    pub bump: u8,
    pub authority: Pubkey,
}

#[account]
#[derive(InitSpace, Debug)]
pub struct TokenRaffle {
    pub authority: Pubkey,
    pub token_mint: Pubkey,
    pub current_round_id: Option<u32>,
    pub current_round_status: RoundStatus,
    pub current_round_end_time: Option<i64>,
    pub total_rounds: u32,
    #[max_len(20)]
    pub pending_rounds: Vec<u32>,  // Rounds waiting to be processed by TukTuk and VRF
    pub bump: u8,
}

#[account]
#[derive(InitSpace, Debug)] 
pub struct Round {
    pub initialized: bool,
    pub token_raffle: Pubkey, //connection with TokenRaffle struct
    pub round_id: u32,
    pub status: RoundStatus,
    pub start_time: i64,
    pub end_time: i64,
    pub prize_amount: u64,
    pub commission_balance: u64,
    pub purchases_count: u32,
    pub total_tickets: u32,
    pub winner_ticket_index: Option<u32>,
    pub winner_purchase_index: Option<u32>,
    pub winner_address: Option<Pubkey>,
    pub prize_claimed: bool,
    pub bump: u8,
}

#[account(zero_copy)]
#[derive(Debug)]
pub struct RoundTickets {
    pub round: Pubkey, //connection with Round struct
    pub cumulative_tickets_1: [u32; 1024],
    pub cumulative_tickets_2: [u32; 1024],
    pub len: u32,
    pub bump: u8,
    pub padding: [u8; 3],
}

impl RoundTickets {
    //TEST
    // Helper для читання як один масив
    pub fn get_tickets(&self) -> Vec<u32> {
        let len = self.len as usize;
        
        if len == 0 {
            return Vec::new();
        }
        
        if len <= 1024 {
            // Всі tickets в першому масиві
            self.cumulative_tickets_1[..len].to_vec()
        } else {
            // Tickets розподілені по обох масивах
            let mut result = Vec::with_capacity(len);
            result.extend_from_slice(&self.cumulative_tickets_1);
            result.extend_from_slice(&self.cumulative_tickets_2[..(len - 1024)]);
            result
        }
    }
    //TEST
    
    pub fn get_ticket_at(&self, index: usize) -> u32 {
        if index < 1024 {
            self.cumulative_tickets_1[index]
        } else {
            self.cumulative_tickets_2[index - 1024]
        }
    }
    
    pub fn add_cumulative(&mut self, cumulative: u32) -> Result<()> {
        let idx = self.len as usize;
        
        if idx < 1024 {
            self.cumulative_tickets_1[idx] = cumulative;
        } else {
            self.cumulative_tickets_2[idx - 1024] = cumulative;
        }
        
        self.len += 1;
        Ok(())
    }
    
    // Binary search
    pub fn partition_point<F>(&self, mut pred: F) -> usize 
    where
        F: FnMut(u32) -> bool
    {
        let len = self.len as usize;
        let mut left = 0;
        let mut right = len;
        
        while left < right {
            let mid = left + (right - left) / 2;
            let value = self.get_ticket_at(mid);
            
            if pred(value) {
                left = mid + 1;
            } else {
                right = mid;
            }
        }
        
        left
    }
}

#[account]
#[derive(InitSpace)]
pub struct RoundTicketsPurchase {
    pub round: Pubkey, //connection with Round struct
    pub player: Pubkey,
    pub purchase_index: u32,
    pub tickets_count: u32,
    pub bump: u8,
}

#[account]
#[derive(InitSpace, Debug)]
pub struct ClaimPrizeTicket {
    pub winner: Pubkey,
    pub prize_amount: u64,
    pub commission_amount: u64,
    pub created_at: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct VRFContext {
    pub token_mint: Pubkey,  
    pub round_id: u32,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, InitSpace, Debug)]
pub enum RoundStatus {
    Open, // 0 - Round is open for tickets buying
    Completed, // 1 - Round closed, winner picked
}

// Events
#[event]
pub struct TicketPurchased {
    pub token: Pubkey,
    pub round_id: u32,
    pub buyer: Pubkey,
    pub count: u32,
    pub total_amount: u64,
    pub prize_amount: u64,
    pub commission_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct FirstTicketBonusAwarded {
    pub token: Pubkey,
    pub round_id: u32,
    pub buyer: Pubkey,
    pub timestamp: i64,
    pub round_start_time: i64,
    pub round_end_time: i64,
}

#[event]
pub struct StatusChanged {
    pub round_id: u32,
    pub old_status: RoundStatus,
    pub new_status: RoundStatus,
    pub timestamp: i64,
}

#[event]
pub struct AllRequestsCompleted {}

#[event]
pub struct PrizeClaimed {
    pub round_id: u32,
    pub winner: Pubkey,
}

// Error codes
#[error_code]
pub enum RaffleError {
    #[msg("Prize already claimed")]
    AlreadyClaimed,
    
    #[msg("All VRF request accounts is used")]
    AllRequestsCompleted,

    #[msg("Cannot change status of completed round")]
    CannotChangeCompletedRound,

    #[msg("Insufficient funds")]
    InsufficientFunds,

    #[msg("Insufficient slippage")]
    InsufficientSlippage,

    #[msg("Insufficient vault balance")]
    InsufficientVaultBalance,

    #[msg("Invalid account size")]
    InvalidAccountSize,

    #[msg("Invalid amount")]
    InvalidAmount,

    #[msg("Invalid beneficiary")]
    InvalidBeneficiary,

    #[msg("Invalid price feed account")]
    InvalidFeedAccount,

    #[msg("Invalid purchase index")]
    InvalidPurchaseIndex,

    #[msg("Invalid ticket count")]
    InvalidTicketCount,

    #[msg("Invalid ticket price")]
    InvalidTicketPrice,

    #[msg("Not enough funds to play")]
    NotEnoughFundsToPlay,

    #[msg("Not enough remaining accounts")]
    NotEnoughRemainingAccounts,

    #[msg("Not enough seeds")]
    NotEnoughSeeds,

    #[msg("Caller is not the winner of this round")]
    NotTheWinner,

    #[msg("Oracle returned invalid or stale data")]
    OracleError,

    #[msg("Prize for this round has already been claimed")]
    PrizeAlreadyClaimed,

    #[msg("Round not available")]
    RoundNotAvailable,

    #[msg("Round has not been completed yet")]
    RoundNotCompleted,

    #[msg("Round not created yet")]
    RoundNotCreated,

    #[msg("Round not ended yet")]
    RoundNotEndedYet,

    #[msg("Round not initialized")]
    RoundNotInitialized,

    #[msg("Round not open")]
    RoundNotOpen,

    #[msg("Round tickets not initialized")]
    RoundTicketsNotInitialized,

    #[msg("Seed mismatch")]
    SeedMismatch,

    #[msg("Ticket not found")]
    TicketNotFound,

    #[msg("Unauthorized")]
    Unauthorized,

    #[msg("Winner address already set")]
    WinnerAlreadySet,
}
