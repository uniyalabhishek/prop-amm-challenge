use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use anyhow::Result;
use prop_amm_shared::normalizer::{
    after_swap as normalizer_after_swap, compute_swap as normalizer_swap,
};
use prop_amm_sim::runner;

const BPS_DENOMINATOR: u128 = 10_000;
const Q32: u128 = 1u128 << 32;
const MAX_FEE_BPS: u128 = 1_200;
const MIN_FEE_BPS: i128 = 5;
const EWMA_ALPHA_NUM: u128 = 15;
const EWMA_ALPHA_DEN: u128 = 16;
const OFFSET_DECAY_NUM: i64 = 31;
const OFFSET_DECAY_DEN: i64 = 32;

static BASE_FEE_BPS: AtomicU64 = AtomicU64::new(25);
static VOL_SENSITIVITY: AtomicU64 = AtomicU64::new(12_000);
static TAIL_FEE_BPS: AtomicU64 = AtomicU64::new(400);
static THRESHOLD_BPS: AtomicU64 = AtomicU64::new(40);
static RETAIL_UP_BPS: AtomicU64 = AtomicU64::new(0);
static GAP_DOWN_BPS: AtomicU64 = AtomicU64::new(1);
static OFFSET_CAP_BPS: AtomicU64 = AtomicU64::new(10);
static TOX_COEF: AtomicU64 = AtomicU64::new(0);
static STALE_COEF: AtomicU64 = AtomicU64::new(0);
static DIR_COEF: AtomicU64 = AtomicU64::new(0);
static DIR_IMPACT_MULT: AtomicU64 = AtomicU64::new(2);
static PHAT_ALPHA_BPS: AtomicU64 = AtomicU64::new(2_500);
static DIR_DECAY_BPS: AtomicU64 = AtomicU64::new(8_000);

#[derive(Clone, Copy, Debug)]
struct Params {
    base_fee_bps: u16,
    vol_sensitivity: u16,
    tail_fee_bps: u16,
    threshold_bps: u16,
    retail_up_bps: u16,
    gap_down_bps: u16,
    offset_cap_bps: u16,
    tox_coef: u16,
    stale_coef: u16,
    dir_coef: u16,
    dir_impact_mult: u16,
    phat_alpha_bps: u16,
    dir_decay_bps: u16,
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
fn compute_dynamic_fee_bps(storage: &[u8], side: u8, reserve_x: u128, reserve_y: u128) -> u128 {
    let base = BASE_FEE_BPS.load(AtomicOrdering::Relaxed) as u128;
    let sensitivity = VOL_SENSITIVITY.load(AtomicOrdering::Relaxed) as u128;
    let ewma_abs_return_q32 = read_u64(storage, 16) as u128;
    let vol_add = ewma_abs_return_q32.saturating_mul(sensitivity) / Q32;
    let fee_offset = read_i64(storage, 24) as i128;

    let mut fee = base as i128 + vol_add as i128 + fee_offset;

    // Directional skew from cumulative buy/sell pressure.
    let dir_state_q32 = read_i64(storage, 32) as i128;
    let dir_coef = DIR_COEF.load(AtomicOrdering::Relaxed) as i128;
    let dir_adjust = dir_state_q32.saturating_mul(dir_coef) / Q32 as i128;
    if side == 0 {
        fee = fee.saturating_add(dir_adjust);
    } else if side == 1 {
        fee = fee.saturating_sub(dir_adjust);
    }

    // Toxicity + stale direction skew versus estimated fair price pHat.
    let p_hat_q32 = read_u64(storage, 8) as u128;
    if p_hat_q32 > 0 && reserve_x > 0 {
        let spot_q32 = reserve_y.saturating_mul(Q32) / reserve_x;
        let tox_q32 = spot_q32.abs_diff(p_hat_q32).saturating_mul(Q32) / p_hat_q32;
        let tox_coef = TOX_COEF.load(AtomicOrdering::Relaxed) as u128;
        let stale_coef = STALE_COEF.load(AtomicOrdering::Relaxed) as u128;
        let tox_add = tox_q32.saturating_mul(tox_coef) / Q32;
        let stale = tox_q32.saturating_mul(stale_coef) / Q32;
        fee = fee.saturating_add(tox_add as i128);

        if spot_q32 >= p_hat_q32 {
            if side == 0 {
                fee = fee.saturating_add(stale as i128);
            } else {
                fee = fee.saturating_sub((stale / 2) as i128);
            }
        } else if side == 1 {
            fee = fee.saturating_add(stale as i128);
        } else {
            fee = fee.saturating_sub((stale / 2) as i128);
        }
    }

    fee.clamp(MIN_FEE_BPS, MAX_FEE_BPS as i128) as u128
}

fn dynamic_piecewise_swap(data: &[u8]) -> u64 {
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

    let fee_low_bps = compute_dynamic_fee_bps(storage, side, reserve_x, reserve_y);
    let tail_fee_bps = TAIL_FEE_BPS.load(AtomicOrdering::Relaxed) as u128;
    let fee_high_bps = fee_low_bps.saturating_add(tail_fee_bps).min(MAX_FEE_BPS);
    let threshold_bps = THRESHOLD_BPS.load(AtomicOrdering::Relaxed) as u128;

    let (reserve_in, reserve_out) = if side == 0 {
        (reserve_y, reserve_x)
    } else if side == 1 {
        (reserve_x, reserve_y)
    } else {
        return 0;
    };

    let threshold_input = reserve_in.saturating_mul(threshold_bps) / BPS_DENOMINATOR;
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

fn dynamic_after_swap(data: &[u8], storage: &mut [u8]) {
    if data.len() < 42 {
        return;
    }

    let side = data[1];
    let input_amount = u64::from_le_bytes(data[2..10].try_into().unwrap());
    let reserve_x = u64::from_le_bytes(data[18..26].try_into().unwrap());
    let reserve_y = u64::from_le_bytes(data[26..34].try_into().unwrap());
    let step = u64::from_le_bytes(data[34..42].try_into().unwrap());
    if reserve_x == 0 || reserve_y == 0 {
        return;
    }

    let current_spot_q32 = spot_price_q32(reserve_x, reserve_y);
    let last_step = read_u64(storage, 0);
    let p_hat_q32 = read_u64(storage, 8);
    let prev_ewma_q32 = read_u64(storage, 16) as u128;
    let mut fee_offset_bps = read_i64(storage, 24);
    let mut dir_state_q32 = read_i64(storage, 32);

    if p_hat_q32 == 0 {
        write_u64(storage, 0, step);
        write_u64(storage, 8, current_spot_q32);
        write_u64(storage, 16, prev_ewma_q32 as u64);
        write_i64(storage, 24, fee_offset_bps);
        write_i64(storage, 32, dir_state_q32);
        return;
    }

    if step != last_step {
        if last_step > 0 {
            let missed_steps = step.saturating_sub(last_step).saturating_sub(1) as i64;
            let gap_down = GAP_DOWN_BPS.load(AtomicOrdering::Relaxed) as i64;
            fee_offset_bps = fee_offset_bps.saturating_sub(missed_steps.saturating_mul(gap_down));
        }
        fee_offset_bps = fee_offset_bps.saturating_mul(OFFSET_DECAY_NUM) / OFFSET_DECAY_DEN;

        let diff = current_spot_q32.abs_diff(p_hat_q32) as u128;
        let ret_q32 = diff.saturating_mul(Q32) / p_hat_q32 as u128;
        let next_ewma_q32 = (prev_ewma_q32.saturating_mul(EWMA_ALPHA_NUM) / EWMA_ALPHA_DEN)
            .saturating_add(ret_q32 / EWMA_ALPHA_DEN);
        write_u64(storage, 16, next_ewma_q32 as u64);

        let alpha_bps = PHAT_ALPHA_BPS.load(AtomicOrdering::Relaxed) as u128;
        let next_p_hat = (p_hat_q32 as u128)
            .saturating_mul(BPS_DENOMINATOR - alpha_bps)
            .saturating_add((current_spot_q32 as u128).saturating_mul(alpha_bps))
            / BPS_DENOMINATOR;
        write_u64(storage, 8, next_p_hat as u64);
        write_u64(storage, 0, step);

        let dir_decay_bps = DIR_DECAY_BPS.load(AtomicOrdering::Relaxed) as i64;
        dir_state_q32 = dir_state_q32.saturating_mul(dir_decay_bps) / BPS_DENOMINATOR as i64;
    } else {
        let retail_up = RETAIL_UP_BPS.load(AtomicOrdering::Relaxed) as i64;
        fee_offset_bps = fee_offset_bps.saturating_add(retail_up);
    }

    // Update directional pressure from trade size ratio.
    let reserve_in_after = if side == 0 { reserve_y } else { reserve_x };
    if reserve_in_after > 0 {
        let ratio_q32 = (input_amount as u128).saturating_mul(Q32) / reserve_in_after as u128;
        let impact_mult = DIR_IMPACT_MULT.load(AtomicOrdering::Relaxed) as i128;
        let impact = ratio_q32.saturating_mul(impact_mult as u128) as i64;
        if side == 0 {
            dir_state_q32 = dir_state_q32.saturating_add(impact);
        } else if side == 1 {
            dir_state_q32 = dir_state_q32.saturating_sub(impact);
        }
    }

    let offset_cap = OFFSET_CAP_BPS.load(AtomicOrdering::Relaxed) as i64;
    fee_offset_bps = fee_offset_bps.clamp(-offset_cap, offset_cap);
    dir_state_q32 = dir_state_q32.clamp(-(Q32 as i64), Q32 as i64);

    write_i64(storage, 24, fee_offset_bps);
    write_i64(storage, 32, dir_state_q32);
}

fn set_params(params: Params) {
    BASE_FEE_BPS.store(params.base_fee_bps as u64, AtomicOrdering::Relaxed);
    VOL_SENSITIVITY.store(params.vol_sensitivity as u64, AtomicOrdering::Relaxed);
    TAIL_FEE_BPS.store(params.tail_fee_bps as u64, AtomicOrdering::Relaxed);
    THRESHOLD_BPS.store(params.threshold_bps as u64, AtomicOrdering::Relaxed);
    RETAIL_UP_BPS.store(params.retail_up_bps as u64, AtomicOrdering::Relaxed);
    GAP_DOWN_BPS.store(params.gap_down_bps as u64, AtomicOrdering::Relaxed);
    OFFSET_CAP_BPS.store(params.offset_cap_bps as u64, AtomicOrdering::Relaxed);
    TOX_COEF.store(params.tox_coef as u64, AtomicOrdering::Relaxed);
    STALE_COEF.store(params.stale_coef as u64, AtomicOrdering::Relaxed);
    DIR_COEF.store(params.dir_coef as u64, AtomicOrdering::Relaxed);
    DIR_IMPACT_MULT.store(params.dir_impact_mult as u64, AtomicOrdering::Relaxed);
    PHAT_ALPHA_BPS.store(params.phat_alpha_bps as u64, AtomicOrdering::Relaxed);
    DIR_DECAY_BPS.store(params.dir_decay_bps as u64, AtomicOrdering::Relaxed);
}

fn env_parse<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn main() -> Result<()> {
    let params = Params {
        base_fee_bps: env_parse("BASE_FEE_BPS", 25u16),
        vol_sensitivity: env_parse("VOL_SENSITIVITY", 12_000u16),
        tail_fee_bps: env_parse("TAIL_FEE_BPS", 400u16),
        threshold_bps: env_parse("THRESHOLD_BPS", 40u16),
        retail_up_bps: env_parse("RETAIL_UP_BPS", 0u16),
        gap_down_bps: env_parse("GAP_DOWN_BPS", 1u16),
        offset_cap_bps: env_parse("OFFSET_CAP_BPS", 10u16),
        tox_coef: env_parse("TOX_COEF", 0u16),
        stale_coef: env_parse("STALE_COEF", 0u16),
        dir_coef: env_parse("DIR_COEF", 0u16),
        dir_impact_mult: env_parse("DIR_IMPACT_MULT", 2u16),
        phat_alpha_bps: env_parse("PHAT_ALPHA_BPS", 2_500u16),
        dir_decay_bps: env_parse("DIR_DECAY_BPS", 8_000u16),
    };

    let sims = env_parse("EVAL_SIMS", 1_000u32);
    let steps = env_parse("EVAL_STEPS", 10_000u32);
    let workers = Some(env_parse("EVAL_WORKERS", 8usize));
    let seed_start = env_parse("EVAL_SEED_START", 0u64);
    let seed_stride = env_parse("EVAL_SEED_STRIDE", 1u64);

    set_params(params);
    let result = runner::run_default_batch_native_seeded(
        dynamic_piecewise_swap,
        Some(dynamic_after_swap),
        normalizer_swap,
        Some(normalizer_after_swap),
        sims,
        steps,
        workers,
        seed_start,
        seed_stride,
    )?;

    println!(
        "params: base={} vol={} tail={} threshold={} retail_up={} gap_down={} cap={} tox={} stale={} dir={} impact={} phat_alpha={} dir_decay={}",
        params.base_fee_bps,
        params.vol_sensitivity,
        params.tail_fee_bps,
        params.threshold_bps,
        params.retail_up_bps,
        params.gap_down_bps,
        params.offset_cap_bps,
        params.tox_coef,
        params.stale_coef,
        params.dir_coef,
        params.dir_impact_mult,
        params.phat_alpha_bps,
        params.dir_decay_bps,
    );
    println!(
        "eval: sims={} steps={} seed_start={} seed_stride={} avg_edge={:.2} total_edge={:.2}",
        sims,
        steps,
        seed_start,
        seed_stride,
        result.avg_edge(),
        result.total_edge,
    );

    Ok(())
}
