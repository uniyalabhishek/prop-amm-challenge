use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};
use prop_amm_submission_sdk::{set_return_data_bytes, set_return_data_u64, set_storage};

/// Displayed on the leaderboard.
const NAME: &str = "Adaptive Exp";
const MODEL_USED: &str = "None";
const STORAGE_SIZE: usize = 1024;
const NANO: f64 = 1_000_000_000.0;

// Exponential curve parameters
const DEFAULT_FEE_BPS: f64 = 55.0;
const ALPHA: f64 = 0.30;
const WARMUP_MIN: u32 = 8;

// Storage layout:
// [0..8]:   last implied price (f64)
// [8..16]:  (reserved)
// [16..24]: last step (u64)
// [24..28]: step-transition observation count (u32)
// [28..36]: sum of squared log-returns per step (f64)
// [36..40]: observation count for running avg (u32)

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

// --- Storage helpers ---

fn rd_f64(s: &[u8], o: usize) -> f64 {
    f64::from_le_bytes([s[o], s[o+1], s[o+2], s[o+3], s[o+4], s[o+5], s[o+6], s[o+7]])
}

fn wr_f64(s: &mut [u8], o: usize, v: f64) {
    s[o..o+8].copy_from_slice(&v.to_le_bytes());
}

fn rd_u64(s: &[u8], o: usize) -> u64 {
    u64::from_le_bytes([s[o], s[o+1], s[o+2], s[o+3], s[o+4], s[o+5], s[o+6], s[o+7]])
}

fn wr_u64(s: &mut [u8], o: usize, v: u64) {
    s[o..o+8].copy_from_slice(&v.to_le_bytes());
}

fn rd_u32(s: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([s[o], s[o+1], s[o+2], s[o+3]])
}

fn wr_u32(s: &mut [u8], o: usize, v: u32) {
    s[o..o+4].copy_from_slice(&v.to_le_bytes());
}

// --- Dynamic fee from estimated σ ---

fn get_fee_bps(storage: &[u8; STORAGE_SIZE]) -> f64 {
    let n = rd_u32(storage, 36);
    if n < WARMUP_MIN {
        return DEFAULT_FEE_BPS;
    }

    let sum_sq = rd_f64(storage, 28);
    if !sum_sq.is_finite() || sum_sq <= 0.0 {
        return DEFAULT_FEE_BPS;
    }

    let sigma = (sum_sq / n as f64).sqrt();
    if !sigma.is_finite() || sigma <= 0.0 {
        return DEFAULT_FEE_BPS;
    }

    // For σ < 0.005 (50 bps): fee stays near default 55bp
    // For σ > 0.005: fee ramps up to protect against increased arb losses
    let sigma_bps = sigma * 10000.0;
    let fee = if sigma_bps > 50.0 {
        55.0 + (sigma_bps - 50.0) * 3.0
    } else {
        55.0
    };

    fee.max(50.0).min(200.0)
}

// --- Core swap computation ---

pub fn compute_swap(data: &[u8]) -> u64 {
    let decoded: ComputeSwapInstruction = match wincode::deserialize(data) {
        Ok(d) => d,
        Err(_) => return 0,
    };

    let input = decoded.input_amount as f64 / NANO;
    let rx = decoded.reserve_x as f64 / NANO;
    let ry = decoded.reserve_y as f64 / NANO;

    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 {
        return 0;
    }

    let fee_bps = get_fee_bps(&decoded.storage);
    let gamma = (10000.0 - fee_bps) / 10000.0;

    // Exponential curve: output = α * reserve_out * (1 - exp(-γ * input / (α * reserve_in)))
    // Same marginal price as CP at zero input, but more concave → less arb extraction
    let (res_in, res_out) = match decoded.side {
        0 => (ry, rx), // Buy X: input Y, output X
        1 => (rx, ry), // Sell X: input X, output Y
        _ => return 0,
    };

    let u = gamma * input / (res_in * ALPHA);
    let output = ALPHA * res_out * (1.0 - (-u).exp());

    if output <= 0.0 || !output.is_finite() {
        return 0;
    }

    // Cap at 99.9% of reserves to ensure validity
    let capped = output.min(res_out * 0.999);
    let scaled = (capped * NANO).floor();

    if scaled <= 0.0 || scaled >= u64::MAX as f64 {
        return 0;
    }

    scaled as u64
}

// --- afterSwap: estimate σ from step-transition price changes ---

pub fn after_swap(data: &[u8], storage: &mut [u8]) {
    if data.len() < 42 {
        return;
    }

    // Decode afterSwap fields
    let rx_raw = u64::from_le_bytes([
        data[18], data[19], data[20], data[21], data[22], data[23], data[24], data[25],
    ]);
    let ry_raw = u64::from_le_bytes([
        data[26], data[27], data[28], data[29], data[30], data[31], data[32], data[33],
    ]);
    let step = u64::from_le_bytes([
        data[34], data[35], data[36], data[37], data[38], data[39], data[40], data[41],
    ]);

    if rx_raw == 0 || ry_raw == 0 {
        return;
    }

    let current_price = ry_raw as f64 / rx_raw as f64;
    let last_price = rd_f64(storage, 0);
    let last_step = rd_u64(storage, 16);

    // Only estimate σ on STEP TRANSITIONS (filters out same-step retail noise)
    // At each new step, the arb corrects price to ~fair, so the price change
    // between consecutive step-transitions reflects fair price movement (= σ)
    if last_price > 0.0 && last_price.is_finite() && step > last_step {
        let dt = (step - last_step) as f64;
        let log_ret = (current_price / last_price).ln();
        let sigma_sq_per_step = (log_ret * log_ret) / dt;

        if sigma_sq_per_step.is_finite() && sigma_sq_per_step >= 0.0 {
            // Running sum for simple average (σ is constant per simulation)
            let sum = rd_f64(storage, 28);
            let n = rd_u32(storage, 36);
            wr_f64(storage, 28, sum + sigma_sq_per_step);
            wr_u32(storage, 36, n + 1);
        }
    }

    // Always update last observed price and step
    wr_f64(storage, 0, current_price);
    wr_u64(storage, 16, step);

    let _ = set_storage(storage);
}
