use std::cmp::Ordering;
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

static BASE_FEE_BPS: AtomicU64 = AtomicU64::new(40);
static VOL_SENSITIVITY: AtomicU64 = AtomicU64::new(0);
static TAIL_FEE_BPS: AtomicU64 = AtomicU64::new(0);
static THRESHOLD_BPS: AtomicU64 = AtomicU64::new(20);
static RETAIL_UP_BPS: AtomicU64 = AtomicU64::new(0);
static GAP_DOWN_BPS: AtomicU64 = AtomicU64::new(0);
static OFFSET_CAP_BPS: AtomicU64 = AtomicU64::new(30);
static INVENTORY_SKEW_K: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
struct Params {
    base_fee_bps: u16,
    vol_sensitivity: u16,
    tail_fee_bps: u16,
    threshold_bps: u16,
    retail_up_bps: u16,
    gap_down_bps: u16,
    offset_cap_bps: u16,
    inventory_skew_k: u16,
}

#[derive(Clone, Debug)]
struct EvalResult {
    params: Params,
    train_avg_edge: f64,
    holdout_avg_edge: f64,
    combined_avg_edge: f64,
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

    let anchor_price_q32 = read_u64(storage, 8) as u128;
    if anchor_price_q32 > 0 && reserve_x > 0 {
        let current_price_q32 = reserve_y.saturating_mul(Q32) / reserve_x;
        let current_q64 = current_price_q32.saturating_mul(Q32);
        let anchor_q64 = anchor_price_q32.saturating_mul(Q32);
        let skew_q32_signed = ((current_q64 as i128) - (anchor_q64 as i128)) / anchor_price_q32 as i128;
        let inventory_k = INVENTORY_SKEW_K.load(AtomicOrdering::Relaxed) as i128;
        let skew_adjust_bps = skew_q32_signed.saturating_mul(inventory_k) / Q32 as i128;
        if side == 0 {
            fee = fee.saturating_add(skew_adjust_bps);
        } else if side == 1 {
            fee = fee.saturating_sub(skew_adjust_bps);
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
            let gap_down = GAP_DOWN_BPS.load(AtomicOrdering::Relaxed) as i64;
            fee_offset_bps = fee_offset_bps.saturating_sub(missed_steps.saturating_mul(gap_down));
        }
        fee_offset_bps = fee_offset_bps.saturating_mul(OFFSET_DECAY_NUM) / OFFSET_DECAY_DEN;

        let next_ewma_q32 = if last_price_q32 > 0 {
            let diff = current_price_q32.abs_diff(last_price_q32) as u128;
            let ret_q32 = diff.saturating_mul(Q32) / last_price_q32 as u128;
            (prev_ewma_q32.saturating_mul(EWMA_ALPHA_NUM) / EWMA_ALPHA_DEN)
                .saturating_add(ret_q32 / EWMA_ALPHA_DEN)
        } else {
            prev_ewma_q32
        };

        write_u64(storage, 0, step);
        write_u64(storage, 8, current_price_q32);
        write_u64(storage, 16, next_ewma_q32 as u64);
    } else {
        let retail_up = RETAIL_UP_BPS.load(AtomicOrdering::Relaxed) as i64;
        fee_offset_bps = fee_offset_bps.saturating_add(retail_up);
    }

    let offset_cap = OFFSET_CAP_BPS.load(AtomicOrdering::Relaxed) as i64;
    fee_offset_bps = fee_offset_bps.clamp(-offset_cap, offset_cap);
    write_i64(storage, 24, fee_offset_bps);
}

fn set_params(params: Params) {
    BASE_FEE_BPS.store(params.base_fee_bps as u64, AtomicOrdering::Relaxed);
    VOL_SENSITIVITY.store(params.vol_sensitivity as u64, AtomicOrdering::Relaxed);
    TAIL_FEE_BPS.store(params.tail_fee_bps as u64, AtomicOrdering::Relaxed);
    THRESHOLD_BPS.store(params.threshold_bps as u64, AtomicOrdering::Relaxed);
    RETAIL_UP_BPS.store(params.retail_up_bps as u64, AtomicOrdering::Relaxed);
    GAP_DOWN_BPS.store(params.gap_down_bps as u64, AtomicOrdering::Relaxed);
    OFFSET_CAP_BPS.store(params.offset_cap_bps as u64, AtomicOrdering::Relaxed);
    INVENTORY_SKEW_K.store(params.inventory_skew_k as u64, AtomicOrdering::Relaxed);
}

fn evaluate_params(
    params: Params,
    sims: u32,
    steps: u32,
    workers: Option<usize>,
    train_seed_start: u64,
    holdout_seed_start: u64,
) -> Result<EvalResult> {
    set_params(params);
    let train = runner::run_default_batch_native_seeded(
        dynamic_piecewise_swap,
        Some(dynamic_after_swap),
        normalizer_swap,
        Some(normalizer_after_swap),
        sims,
        steps,
        workers,
        train_seed_start,
        1,
    )?;

    set_params(params);
    let holdout = runner::run_default_batch_native_seeded(
        dynamic_piecewise_swap,
        Some(dynamic_after_swap),
        normalizer_swap,
        Some(normalizer_after_swap),
        sims,
        steps,
        workers,
        holdout_seed_start,
        1,
    )?;

    let train_avg_edge = train.avg_edge();
    let holdout_avg_edge = holdout.avg_edge();
    let combined_avg_edge = 0.5 * (train_avg_edge + holdout_avg_edge);

    Ok(EvalResult {
        params,
        train_avg_edge,
        holdout_avg_edge,
        combined_avg_edge,
    })
}

fn top_k(mut results: Vec<EvalResult>, k: usize) -> Vec<EvalResult> {
    results.sort_by(|a, b| {
        b.combined_avg_edge
            .partial_cmp(&a.combined_avg_edge)
            .unwrap_or(Ordering::Equal)
    });
    results.truncate(k);
    results
}

fn print_results(title: &str, results: &[EvalResult]) {
    println!("\n{}", title);
    for (idx, result) in results.iter().enumerate() {
        println!(
            "#{:02} combined={:8.2} train={:8.2} holdout={:8.2} | base={} vol_k={} tail={} threshold_bps={} retail_up={} gap_down={} cap={} inv_k={}",
            idx + 1,
            result.combined_avg_edge,
            result.train_avg_edge,
            result.holdout_avg_edge,
            result.params.base_fee_bps,
            result.params.vol_sensitivity,
            result.params.tail_fee_bps,
            result.params.threshold_bps,
            result.params.retail_up_bps,
            result.params.gap_down_bps,
            result.params.offset_cap_bps,
            result.params.inventory_skew_k,
        );
    }
}

fn env_parse<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn main() -> Result<()> {
    let quick_sims = env_parse("SWEEP_SIMS", 40u32);
    let quick_steps = env_parse("SWEEP_STEPS", 2_500u32);
    let workers = Some(env_parse("SWEEP_WORKERS", 8usize));
    let train_seed_start = env_parse("SWEEP_TRAIN_START", 0u64);
    let holdout_seed_start = env_parse("SWEEP_HOLDOUT_START", 100_000u64);

    println!(
        "Running dynamic quick sweep: sims={} steps={} workers={:?} train_start={} holdout_start={}",
        quick_sims, quick_steps, workers, train_seed_start, holdout_seed_start,
    );

    // Stage 1: Find strong base fee + volatility sensitivity without tail steepening.
    let base_fee_candidates = [20u16, 30, 40, 50, 60, 70, 80];
    let vol_k_candidates = [0u16, 4_000, 8_000, 12_000, 16_000, 20_000, 28_000, 36_000];
    let mut stage1 = Vec::new();
    for base_fee_bps in base_fee_candidates {
        for vol_sensitivity in vol_k_candidates {
            stage1.push(evaluate_params(
                Params {
                    base_fee_bps,
                    vol_sensitivity,
                    tail_fee_bps: 0,
                    threshold_bps: 20,
                    retail_up_bps: 0,
                    gap_down_bps: 0,
                    offset_cap_bps: 30,
                    inventory_skew_k: 0,
                },
                quick_sims,
                quick_steps,
                workers,
                train_seed_start,
                holdout_seed_start,
            )?);
        }
    }
    let top_stage1 = top_k(stage1, 6);
    print_results("Top base+vol candidates (stage 1)", &top_stage1);

    // Stage 2: Add a tail fee regime for larger trades.
    let tail_candidates = [0u16, 40, 80, 120, 160, 240, 320, 500, 700];
    let threshold_candidates = [2u16, 5, 10, 20, 40, 80, 160];
    let mut stage2 = Vec::new();
    for seed in top_stage1.iter().take(4) {
        for tail_fee_bps in tail_candidates {
            for threshold_bps in threshold_candidates {
                stage2.push(evaluate_params(
                    Params {
                        base_fee_bps: seed.params.base_fee_bps,
                        vol_sensitivity: seed.params.vol_sensitivity,
                        tail_fee_bps,
                        threshold_bps,
                        retail_up_bps: 0,
                        gap_down_bps: 0,
                        offset_cap_bps: 30,
                        inventory_skew_k: 0,
                    },
                    quick_sims,
                    quick_steps,
                    workers,
                    train_seed_start,
                    holdout_seed_start,
                )?);
            }
        }
    }

    let top_stage2 = top_k(stage2, 12);
    print_results("Top dynamic candidates after stage 2", &top_stage2);

    // Stage 3: Activity-based fee offsets to adapt to competitiveness in each simulation.
    let retail_up_candidates = [0u16, 1, 2, 3];
    let gap_down_candidates = [0u16, 1, 2, 3];
    let offset_cap_candidates = [15u16, 30, 60];
    let mut stage3 = Vec::new();
    for seed in top_stage2.iter().take(4) {
        for retail_up_bps in retail_up_candidates {
            for gap_down_bps in gap_down_candidates {
                for offset_cap_bps in offset_cap_candidates {
                    stage3.push(evaluate_params(
                        Params {
                            base_fee_bps: seed.params.base_fee_bps,
                            vol_sensitivity: seed.params.vol_sensitivity,
                            tail_fee_bps: seed.params.tail_fee_bps,
                            threshold_bps: seed.params.threshold_bps,
                            retail_up_bps,
                            gap_down_bps,
                            offset_cap_bps,
                            inventory_skew_k: 0,
                        },
                        quick_sims,
                        quick_steps,
                        workers,
                        train_seed_start,
                        holdout_seed_start,
                    )?);
                }
            }
        }
    }
    let top_stage3 = top_k(stage3, 15);
    print_results("Top adaptive candidates after stage 3", &top_stage3);

    // Stage 4: Add side-specific inventory skewing around the most recent anchor price.
    let inventory_skew_candidates = [0u16, 500, 1_000, 2_000, 4_000, 8_000];
    let mut stage4 = Vec::new();
    for seed in top_stage3.iter().take(4) {
        for inventory_skew_k in inventory_skew_candidates {
            stage4.push(evaluate_params(
                Params {
                    base_fee_bps: seed.params.base_fee_bps,
                    vol_sensitivity: seed.params.vol_sensitivity,
                    tail_fee_bps: seed.params.tail_fee_bps,
                    threshold_bps: seed.params.threshold_bps,
                    retail_up_bps: seed.params.retail_up_bps,
                    gap_down_bps: seed.params.gap_down_bps,
                    offset_cap_bps: seed.params.offset_cap_bps,
                    inventory_skew_k,
                },
                quick_sims,
                quick_steps,
                workers,
                train_seed_start,
                holdout_seed_start,
            )?);
        }
    }
    let top_stage4 = top_k(stage4, 15);
    print_results("Top inventory-skew candidates after stage 4", &top_stage4);

    let deep_sims = env_parse("DEEP_SIMS", 200u32);
    let deep_steps = env_parse("DEEP_STEPS", 10_000u32);
    println!(
        "\nRunning deep eval on top 6: sims={} steps={}",
        deep_sims, deep_steps
    );

    let mut deep_results = Vec::new();
    for quick_best in top_stage4.iter().take(8) {
        deep_results.push(evaluate_params(
            quick_best.params,
            deep_sims,
            deep_steps,
            workers,
            train_seed_start,
            holdout_seed_start,
        )?);
    }
    let top_deep = top_k(deep_results, 8);
    print_results("Top candidates after deep eval", &top_deep);

    Ok(())
}
