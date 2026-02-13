// price_feeds.rs - Price feed integration for Switchboard and Pyth
use anchor_lang::prelude::*;

pub mod switchboard {
    use super::*;
    
    #[derive(AnchorSerialize, AnchorDeserialize, Clone)]
    pub struct SwitchboardDecimal {
        pub mantissa: i128,
        pub scale: u32,
    }
    
    impl SwitchboardDecimal {
        pub fn to_f64(&self) -> f64 {
            (self.mantissa as f64) / (10_f64.powi(self.scale as i32))
        }
    }
    
    #[account(zero_copy)]
    pub struct AggregatorAccountData {
        pub latest_confirmed_round: AggregatorRound,
        // ... other fields
    }
    
    #[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
    pub struct AggregatorRound {
        pub result: SwitchboardDecimal,
        pub round_open_timestamp: i64,
        // ... other fields
    }
    
    pub fn get_price_from_feed(price_feed: &AccountInfo) -> Result<f64> {
        require!(price_feed.data_len() >= 8, crate::LotteryError::InvalidPriceFeed);
        
        let data = price_feed.try_borrow_data()?;
        let aggregator = bytemuck::from_bytes::<AggregatorAccountData>(&data[8..]);
        
        // Check staleness (5 minutes)
        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp - aggregator.latest_confirmed_round.round_open_timestamp < 300,
            crate::LotteryError::StalePriceFeed
        );
        
        Ok(aggregator.latest_confirmed_round.result.to_f64())
    }
}

pub mod pyth {
    use super::*;
    
    pub fn get_price_from_feed(price_feed: &AccountInfo) -> Result<f64> {
        // Pyth integration would go here
        // For now, returning placeholder
        Ok(50000.0) // $50K placeholder for BTC
    }
}

// Price calculation functions
pub fn calculate_ticket_price_in_lamports(
    btc_price_feed: &AccountInfo,
    sol_price_feed: &AccountInfo,
) -> Result<u64> {
    let btc_price_usd = switchboard::get_price_from_feed(btc_price_feed)?;
    let sol_price_usd = switchboard::get_price_from_feed(sol_price_feed)?;
    
    // TICKET_PRICE_BTC_SATOSHIS = 5000 (0.00005 BTC)
    let ticket_value_usd = (crate::TICKET_PRICE_BTC_SATOSHIS as f64 * btc_price_usd) / (10_f64.powi(crate::BTC_DECIMALS as i32));
    let lamports = (ticket_value_usd / sol_price_usd) * (10_f64.powi(9)); // SOL has 9 decimals
    
    Ok(lamports as u64)
}

pub fn calculate_ticket_price_in_tokens(
    btc_price_feed: &AccountInfo,
    token_price_feed: &AccountInfo,
    token_decimals: u8,
) -> Result<u64> {
    let btc_price_usd = switchboard::get_price_from_feed(btc_price_feed)?;
    let token_price_usd = switchboard::get_price_from_feed(token_price_feed)?;
    
    // Calculate ticket value in USD
    let ticket_value_usd = (crate::TICKET_PRICE_BTC_SATOSHIS as f64 * btc_price_usd) / (10_f64.powi(crate::BTC_DECIMALS as i32));
    
    // Convert to token amount
    let tokens = (ticket_value_usd / token_price_usd) * (10_f64.powi(token_decimals as i32));
    
    Ok(tokens as u64)
}