use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};
use prop_amm_submission_sdk::{
    set_return_data_bytes, set_return_data_u64, set_storage, STORAGE_SIZE,
};

const NAME: &str = "Adaptive Piecewise v1";
const MODEL_USED: &str = "GPT-5.3-Codex";

const BPS_DENOMINATOR: u128 = 10_000;
const Q32: u128 = 1u128 << 32;
const MIN_FEE_BPS: i128 = 5;
const MAX_FEE_BPS: i128 = 1_200;

const BASE_FEE_BPS: i128 = 30;
const VOL_SENSITIVITY: u128 = 12_000;
const TAIL_FEE_BPS: u128 = 320;
const THRESHOLD_BPS: u128 = 40;

const RETAIL_UP_BPS: i64 = 1;
const GAP_DOWN_BPS: i64 = 1;
const OFFSET_CAP_BPS: i64 = 15;

const EWMA_ALPHA_NUM: u128 = 15;
const EWMA_ALPHA_DEN: u128 = 16;
const OFFSET_DECAY_NUM: i64 = 31;
const OFFSET_DECAY_DEN: i64 = 32;

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
            if instruction_data.len() >= 42 {
                let mut storage = [0u8; STORAGE_SIZE];
                let copy_len = (instruction_data.len() - 42).min(STORAGE_SIZE);
                storage[..copy_len].copy_from_slice(&instruction_data[42..42 + copy_len]);
                after_swap(instruction_data, &mut storage);
                let _ = set_storage(&storage);
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

#[inline]
fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    if bytes.len() < offset + 8 {
        return 0;
    }
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[inline]
fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    if bytes.len() < offset + 8 {
        return;
    }
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[inline]
fn read_i64(bytes: &[u8], offset: usize) -> i64 {
    read_u64(bytes, offset) as i64
}

#[inline]
fn write_i64(bytes: &mut [u8], offset: usize, value: i64) {
    write_u64(bytes, offset, value as u64);
}

#[inline]
fn spot_price_q32(reserve_x: u64, reserve_y: u64) -> u64 {
    if reserve_x == 0 {
        return 0;
    }
    ((reserve_y as u128).saturating_mul(Q32) / reserve_x as u128) as u64
}

#[inline]
fn compute_fee_low_bps(storage: &[u8]) -> u128 {
    let ewma_abs_return_q32 = read_u64(storage, 16) as u128;
    let vol_add = ewma_abs_return_q32.saturating_mul(VOL_SENSITIVITY) / Q32;
    let fee_offset = read_i64(storage, 24) as i128;
    let fee = BASE_FEE_BPS + vol_add as i128 + fee_offset;
    fee.clamp(MIN_FEE_BPS, MAX_FEE_BPS) as u128
}

pub fn compute_swap(data: &[u8]) -> u64 {
    if data.len() < 25 {
        return 0;
    }

    let side = data[0];
    let input_amount = u64::from_le_bytes(data[1..9].try_into().unwrap()) as u128;
    let reserve_x = u64::from_le_bytes(data[9..17].try_into().unwrap()) as u128;
    let reserve_y = u64::from_le_bytes(data[17..25].try_into().unwrap()) as u128;
    let storage = &data[25..];

    if reserve_x == 0 || reserve_y == 0 || input_amount == 0 {
        return 0;
    }

    let fee_low_bps = compute_fee_low_bps(storage);
    let fee_high_bps = (fee_low_bps.saturating_add(TAIL_FEE_BPS)).min(MAX_FEE_BPS as u128);

    let (reserve_in, reserve_out) = if side == 0 {
        (reserve_y, reserve_x)
    } else if side == 1 {
        (reserve_x, reserve_y)
    } else {
        return 0;
    };

    let threshold_input = reserve_in.saturating_mul(THRESHOLD_BPS) / BPS_DENOMINATOR;
    let low_input = input_amount.min(threshold_input);
    let high_input = input_amount.saturating_sub(low_input);

    let net_low = low_input.saturating_mul(BPS_DENOMINATOR - fee_low_bps) / BPS_DENOMINATOR;
    let net_high = high_input.saturating_mul(BPS_DENOMINATOR - fee_high_bps) / BPS_DENOMINATOR;
    let net_input = net_low.saturating_add(net_high);
    if net_input == 0 {
        return 0;
    }

    let k = reserve_x.saturating_mul(reserve_y);
    if k == 0 {
        return 0;
    }

    match side {
        0 => {
            let new_ry = reserve_y.saturating_add(net_input);
            if new_ry == 0 {
                return 0;
            }
            reserve_out.saturating_sub((k + new_ry - 1) / new_ry) as u64
        }
        1 => {
            let new_rx = reserve_x.saturating_add(net_input);
            if new_rx == 0 {
                return 0;
            }
            reserve_out.saturating_sub((k + new_rx - 1) / new_rx) as u64
        }
        _ => 0,
    }
}

pub fn after_swap(data: &[u8], storage: &mut [u8]) {
    if data.len() < 42 || storage.len() < 32 {
        return;
    }

    let reserve_x = u64::from_le_bytes(data[18..26].try_into().unwrap());
    let reserve_y = u64::from_le_bytes(data[26..34].try_into().unwrap());
    let step = u64::from_le_bytes(data[34..42].try_into().unwrap());
    if reserve_x == 0 || reserve_y == 0 {
        return;
    }

    let current_price_q32 = spot_price_q32(reserve_x, reserve_y);
    let last_step = read_u64(storage, 0);
    let last_price_q32 = read_u64(storage, 8);
    let prev_ewma_q32 = read_u64(storage, 16) as u128;
    let mut fee_offset_bps = read_i64(storage, 24);

    if last_price_q32 == 0 {
        write_u64(storage, 0, step);
        write_u64(storage, 8, current_price_q32);
        write_u64(storage, 16, prev_ewma_q32 as u64);
        write_i64(storage, 24, fee_offset_bps);
        return;
    }

    if step != last_step {
        if last_step > 0 {
            let missed_steps = step.saturating_sub(last_step).saturating_sub(1) as i64;
            fee_offset_bps = fee_offset_bps.saturating_sub(missed_steps.saturating_mul(GAP_DOWN_BPS));
        }
        fee_offset_bps = fee_offset_bps.saturating_mul(OFFSET_DECAY_NUM) / OFFSET_DECAY_DEN;

        let diff = current_price_q32.abs_diff(last_price_q32) as u128;
        let ret_q32 = diff.saturating_mul(Q32) / last_price_q32 as u128;
        let next_ewma_q32 = (prev_ewma_q32.saturating_mul(EWMA_ALPHA_NUM) / EWMA_ALPHA_DEN)
            .saturating_add(ret_q32 / EWMA_ALPHA_DEN);

        write_u64(storage, 0, step);
        write_u64(storage, 8, current_price_q32);
        write_u64(storage, 16, next_ewma_q32 as u64);
    } else {
        fee_offset_bps = fee_offset_bps.saturating_add(RETAIL_UP_BPS);
    }

    fee_offset_bps = fee_offset_bps.clamp(-OFFSET_CAP_BPS, OFFSET_CAP_BPS);
    write_i64(storage, 24, fee_offset_bps);
}
