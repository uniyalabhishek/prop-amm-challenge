use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};
use prop_amm_submission_sdk::{set_return_data_bytes, set_return_data_u64, set_storage};

const NAME: &str = "Dynamic Fee";
const MODEL_USED: &str = "None";
const STORAGE_SIZE: usize = 1024;

// Storage layout:
// [0..8]: last_price as f64 bits
// [8..16]: ema_sigma as f64 bits (estimated per-step σ)
// [16..24]: last_step as u64
// [24..28]: trade_count as u32

// Default fee when no data available
const DEFAULT_FEE_BPS: u128 = 65;
// Fee scaling: fee_bps = SIGMA_MULT * sigma * 10000
// At median sigma ~0.35% = 0.0035: fee = SIGMA_MULT * 0.0035 * 10000
const SIGMA_MULT: f64 = 1.8; // gives ~63 bps at median sigma
const MIN_FEE_BPS: u128 = 8;
const MAX_FEE_BPS: u128 = 300;
const EMA_ALPHA: f64 = 0.05; // EMA decay factor

#[derive(wincode::SchemaRead)]
struct ComputeSwapInstruction {
    side: u8,
    input_amount: u64,
    reserve_x: u64,
    reserve_y: u64,
    storage: [u8; STORAGE_SIZE],
}

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if instruction_data.is_empty() {
        return Ok(());
    }
    match instruction_data[0] {
        0 | 1 => {
            let output = compute_swap(instruction_data);
            set_return_data_u64(output);
        }
        2 => {
            // afterSwap: update volatility estimate
            if instruction_data.len() >= 42 + STORAGE_SIZE {
                let mut storage = [0u8; STORAGE_SIZE];
                storage.copy_from_slice(&instruction_data[42..42 + STORAGE_SIZE]);
                after_swap(instruction_data, &mut storage);
            }
        }
        3 => set_return_data_bytes(NAME.as_bytes()),
        4 => set_return_data_bytes(get_model_used().as_bytes()),
        _ => {}
    }
    Ok(())
}

pub fn get_model_used() -> &'static str {
    MODEL_USED
}

fn read_f64(storage: &[u8], offset: usize) -> f64 {
    let bytes: [u8; 8] = storage[offset..offset + 8].try_into().unwrap_or([0u8; 8]);
    f64::from_le_bytes(bytes)
}

fn write_f64(storage: &mut [u8], offset: usize, val: f64) {
    storage[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn read_u64(storage: &[u8], offset: usize) -> u64 {
    let bytes: [u8; 8] = storage[offset..offset + 8].try_into().unwrap_or([0u8; 8]);
    u64::from_le_bytes(bytes)
}

fn write_u64(storage: &mut [u8], offset: usize, val: u64) {
    storage[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn read_u32(storage: &[u8], offset: usize) -> u32 {
    let bytes: [u8; 4] = storage[offset..offset + 4].try_into().unwrap_or([0u8; 4]);
    u32::from_le_bytes(bytes)
}

fn write_u32(storage: &mut [u8], offset: usize, val: u32) {
    storage[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
}

fn get_fee_bps(storage: &[u8; STORAGE_SIZE]) -> u128 {
    let trade_count = read_u32(storage, 24);
    if trade_count < 5 {
        return DEFAULT_FEE_BPS;
    }
    
    let ema_sigma = read_f64(storage, 8);
    if !ema_sigma.is_finite() || ema_sigma <= 0.0 {
        return DEFAULT_FEE_BPS;
    }
    
    // fee_bps = SIGMA_MULT * sigma * 10000
    let fee = (SIGMA_MULT * ema_sigma * 10000.0) as u128;
    if fee < MIN_FEE_BPS {
        MIN_FEE_BPS
    } else if fee > MAX_FEE_BPS {
        MAX_FEE_BPS
    } else {
        fee
    }
}

pub fn compute_swap(data: &[u8]) -> u64 {
    let decoded: ComputeSwapInstruction = match wincode::deserialize(data) {
        Ok(decoded) => decoded,
        Err(_) => return 0,
    };

    let side = decoded.side;
    let input_amount = decoded.input_amount as u128;
    let reserve_x = decoded.reserve_x as u128;
    let reserve_y = decoded.reserve_y as u128;

    if reserve_x == 0 || reserve_y == 0 {
        return 0;
    }

    let fee_bps = get_fee_bps(&decoded.storage);
    let fee_numerator = 10000u128 - fee_bps;
    let fee_denominator = 10000u128;

    let k = reserve_x * reserve_y;

    match side {
        0 => {
            let net_y = input_amount * fee_numerator / fee_denominator;
            let new_ry = reserve_y + net_y;
            let k_div = (k + new_ry - 1) / new_ry;
            reserve_x.saturating_sub(k_div) as u64
        }
        1 => {
            let net_x = input_amount * fee_numerator / fee_denominator;
            let new_rx = reserve_x + net_x;
            let k_div = (k + new_rx - 1) / new_rx;
            reserve_y.saturating_sub(k_div) as u64
        }
        _ => 0,
    }
}

pub fn after_swap(data: &[u8], storage: &mut [u8]) {
    if data.len() < 42 {
        return;
    }
    
    // Decode afterSwap data
    let reserve_x = u64::from_le_bytes(data[18..26].try_into().unwrap_or([0u8; 8]));
    let reserve_y = u64::from_le_bytes(data[26..34].try_into().unwrap_or([0u8; 8]));
    let step = u64::from_le_bytes(data[34..42].try_into().unwrap_or([0u8; 8]));
    
    if reserve_x == 0 || reserve_y == 0 {
        return;
    }
    
    let current_price = reserve_y as f64 / reserve_x as f64;
    let last_price = read_f64(storage, 0);
    let last_step = read_u64(storage, 16);
    let trade_count = read_u32(storage, 24);
    
    if last_price > 0.0 && last_price.is_finite() && step > last_step {
        let dt = (step - last_step) as f64;
        let log_return = (current_price / last_price).ln();
        let sigma_sq_estimate = (log_return * log_return) / dt;
        let sigma_estimate = sigma_sq_estimate.sqrt();
        
        if sigma_estimate.is_finite() {
            let old_ema = read_f64(storage, 8);
            let new_ema = if old_ema > 0.0 && old_ema.is_finite() {
                EMA_ALPHA * sigma_estimate + (1.0 - EMA_ALPHA) * old_ema
            } else {
                sigma_estimate
            };
            write_f64(storage, 8, new_ema);
        }
    }
    
    write_f64(storage, 0, current_price);
    write_u64(storage, 16, step);
    write_u32(storage, 24, trade_count.wrapping_add(1));
    
    let _ = set_storage(storage);
}
