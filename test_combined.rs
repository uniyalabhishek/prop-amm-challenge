use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};
use prop_amm_submission_sdk::{set_return_data_bytes, set_return_data_u64, set_storage};

const NAME: &str = "Adaptive Exp AMM";
const MODEL_USED: &str = "None";
const STORAGE_SIZE: usize = 1024;

// Default parameters (used during warmup)
const DEFAULT_FEE_BPS: f64 = 55.0;
const DEFAULT_ALPHA: f64 = 0.30;

// Dynamic fee parameters
const MIN_FEE_BPS: f64 = 25.0;
const MAX_FEE_BPS: f64 = 250.0;
const EMA_DECAY: f64 = 0.03; // EMA alpha for sigma estimation
const WARMUP_TRADES: u32 = 3;

// Storage layout (48 bytes used):
// [0..8]:   last_implied_price (f64)
// [8..16]:  ema_sigma_sq (f64) - EMA of squared log returns per step
// [16..24]: last_step (u64)
// [24..28]: trade_count (u32)
// [28..36]: sum_sq_returns (f64) - for warmup estimation
// [36..40]: warmup_count (u32) - number of return observations

const NANO: f64 = 1_000_000_000.0;

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
    if instruction_data.is_empty() { return Ok(()); }
    match instruction_data[0] {
        0 | 1 => {
            let output = compute_swap(instruction_data);
            set_return_data_u64(output);
        }
        2 => {
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

pub fn get_model_used() -> &'static str { MODEL_USED }

fn read_f64(s: &[u8], off: usize) -> f64 {
    f64::from_le_bytes(s[off..off+8].try_into().unwrap_or([0u8; 8]))
}
fn write_f64(s: &mut [u8], off: usize, v: f64) {
    s[off..off+8].copy_from_slice(&v.to_le_bytes());
}
fn read_u64(s: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(s[off..off+8].try_into().unwrap_or([0u8; 8]))
}
fn write_u64(s: &mut [u8], off: usize, v: u64) {
    s[off..off+8].copy_from_slice(&v.to_le_bytes());
}
fn read_u32(s: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(s[off..off+4].try_into().unwrap_or([0u8; 4]))
}
fn write_u32(s: &mut [u8], off: usize, v: u32) {
    s[off..off+4].copy_from_slice(&v.to_le_bytes());
}

fn get_fee_and_alpha(storage: &[u8; STORAGE_SIZE]) -> (f64, f64) {
    let trade_count = read_u32(storage, 24);
    let warmup_count = read_u32(storage, 36);
    
    if warmup_count < WARMUP_TRADES {
        return (DEFAULT_FEE_BPS, DEFAULT_ALPHA);
    }
    
    // Get sigma estimate
    let sigma = if trade_count >= WARMUP_TRADES + 5 {
        // Use EMA after enough observations
        let ema_sq = read_f64(storage, 8);
        if ema_sq > 0.0 && ema_sq.is_finite() {
            ema_sq.sqrt()
        } else {
            return (DEFAULT_FEE_BPS, DEFAULT_ALPHA);
        }
    } else {
        // Use simple average during early phase
        let sum_sq = read_f64(storage, 28);
        let n = warmup_count as f64;
        if n > 0.0 && sum_sq > 0.0 && sum_sq.is_finite() {
            (sum_sq / n).sqrt()
        } else {
            return (DEFAULT_FEE_BPS, DEFAULT_ALPHA);
        }
    };
    
    // Fee formula: roughly proportional to sigma
    // At sigma=0.0035 (median): fee ≈ 55bp
    // At sigma=0.001 (low): fee ≈ 30bp  
    // At sigma=0.007 (high): fee ≈ 150bp
    // fee_bps = base + scale * sigma * 10000
    let fee_bps = 20.0 + 10000.0 * sigma;
    let fee_bps = fee_bps.max(MIN_FEE_BPS).min(MAX_FEE_BPS);
    
    // Alpha: use slightly lower alpha in high vol (more concave = less arb loss)
    // At low sigma: alpha=0.35 (less concave, more retail-friendly depth)
    // At high sigma: alpha=0.25 (more concave, more arb-resistant)
    let alpha = 0.35 - 30.0 * sigma;
    let alpha = alpha.max(0.20).min(0.40);
    
    (fee_bps, alpha)
}

pub fn compute_swap(data: &[u8]) -> u64 {
    let decoded: ComputeSwapInstruction = match wincode::deserialize(data) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    
    let side = decoded.side;
    let input = decoded.input_amount as f64 / NANO;
    let rx = decoded.reserve_x as f64 / NANO;
    let ry = decoded.reserve_y as f64 / NANO;
    
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return 0; }
    
    let (fee_bps, alpha) = get_fee_and_alpha(&decoded.storage);
    let gamma = (10000.0 - fee_bps) / 10000.0;
    
    let output = match side {
        0 => {
            // Buy X: input Y, output X
            let u = gamma * input / (ry * alpha);
            alpha * rx * (1.0 - (-u).exp())
        }
        1 => {
            // Sell X: input X, output Y
            let u = gamma * input / (rx * alpha);
            alpha * ry * (1.0 - (-u).exp())
        }
        _ => return 0,
    };
    
    if output <= 0.0 || !output.is_finite() { return 0; }
    let max_out = match side { 0 => rx, 1 => ry, _ => 0.0 };
    let capped = output.min(max_out * 0.999);
    let scaled = (capped * NANO).floor();
    if scaled <= 0.0 || scaled >= u64::MAX as f64 { return 0; }
    scaled as u64
}

pub fn after_swap(data: &[u8], storage: &mut [u8]) {
    if data.len() < 42 { return; }
    
    let reserve_x = u64::from_le_bytes(data[18..26].try_into().unwrap_or([0u8; 8]));
    let reserve_y = u64::from_le_bytes(data[26..34].try_into().unwrap_or([0u8; 8]));
    let step = u64::from_le_bytes(data[34..42].try_into().unwrap_or([0u8; 8]));
    
    if reserve_x == 0 || reserve_y == 0 { return; }
    
    let current_price = reserve_y as f64 / reserve_x as f64;
    let last_price = read_f64(storage, 0);
    let last_step = read_u64(storage, 16);
    let trade_count = read_u32(storage, 24);
    
    if last_price > 0.0 && last_price.is_finite() && step > last_step {
        let dt = (step - last_step) as f64;
        let log_return = (current_price / last_price).ln();
        let sigma_sq_per_step = (log_return * log_return) / dt;
        
        if sigma_sq_per_step.is_finite() && sigma_sq_per_step >= 0.0 {
            let warmup_count = read_u32(storage, 36);
            
            // Accumulate for warmup estimation
            if warmup_count < 100 {
                let sum_sq = read_f64(storage, 28);
                write_f64(storage, 28, sum_sq + sigma_sq_per_step);
                write_u32(storage, 36, warmup_count + 1);
            }
            
            // Update EMA
            let old_ema = read_f64(storage, 8);
            let new_ema = if old_ema > 0.0 && old_ema.is_finite() {
                EMA_DECAY * sigma_sq_per_step + (1.0 - EMA_DECAY) * old_ema
            } else {
                sigma_sq_per_step
            };
            write_f64(storage, 8, new_ema);
        }
    }
    
    write_f64(storage, 0, current_price);
    write_u64(storage, 16, step);
    write_u32(storage, 24, trade_count.wrapping_add(1));
    
    let _ = set_storage(storage);
}
