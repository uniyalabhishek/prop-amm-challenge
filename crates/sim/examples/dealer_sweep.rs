use std::cmp::Ordering;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use anyhow::Result;
use prop_amm_shared::normalizer::{
    after_swap as normalizer_after_swap, compute_swap as normalizer_swap,
};
use prop_amm_sim::runner;

const BPS_DENOMINATOR: u128 = 10_000;
const Q32: u128 = 1u128 << 32;
const MAX_SPREAD_BPS: i128 = 4_000;
const MIN_SPREAD_BPS: i128 = 5;
const EWMA_ALPHA_NUM: u128 = 15;
const EWMA_ALPHA_DEN: u128 = 16;

static BASE_SPREAD_BPS: AtomicU64 = AtomicU64::new(40);
static VOL_SENSITIVITY: AtomicU64 = AtomicU64::new(0);
static TAIL_EXTRA_BPS: AtomicU64 = AtomicU64::new(300);
static THRESHOLD_BPS: AtomicU64 = AtomicU64::new(40);

#[derive(Clone, Copy, Debug)]
struct Params {
    base_spread_bps: u16,
    vol_sensitivity: u16,
    tail_extra_bps: u16,
    threshold_bps: u16,
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
fn spot_price_q32(reserve_x: u64, reserve_y: u64) -> u64 {
    if reserve_x == 0 {
        return 0;
    }
    ((reserve_y as u128).saturating_mul(Q32) / reserve_x as u128) as u64
}

#[inline]
fn mul_div(a: u128, b: u128, den: u128) -> u128 {
    if den == 0 {
        0
    } else {
        a.saturating_mul(b) / den
    }
}

#[inline]
fn clamp_spread(spread: i128) -> u128 {
    spread.clamp(MIN_SPREAD_BPS, MAX_SPREAD_BPS) as u128
}

fn dealer_swap(data: &[u8]) -> u64 {
    if data.len() < 25 {
        return 0;
    }

    let side = data[0];
    let input_amount = u64::from_le_bytes(data[1..9].try_into().unwrap()) as u128;
    let reserve_x = u64::from_le_bytes(data[9..17].try_into().unwrap()) as u128;
    let reserve_y = u64::from_le_bytes(data[17..25].try_into().unwrap()) as u128;
    let storage = &data[25..];
    if input_amount == 0 || reserve_x == 0 || reserve_y == 0 {
        return 0;
    }

    let anchor_price_q32 = {
        let stored = read_u64(storage, 8);
        if stored == 0 {
            spot_price_q32(reserve_x as u64, reserve_y as u64)
        } else {
            stored
        }
    } as u128;
    if anchor_price_q32 == 0 {
        return 0;
    }

    let base_spread = BASE_SPREAD_BPS.load(AtomicOrdering::Relaxed) as i128;
    let vol_sensitivity = VOL_SENSITIVITY.load(AtomicOrdering::Relaxed) as u128;
    let ewma_abs_return_q32 = read_u64(storage, 16) as u128;
    let vol_add = ewma_abs_return_q32.saturating_mul(vol_sensitivity) / Q32;
    let spread1_bps = clamp_spread(base_spread + vol_add as i128);
    let tail_extra = TAIL_EXTRA_BPS.load(AtomicOrdering::Relaxed) as u128;
    let spread2_bps = clamp_spread(spread1_bps as i128 + tail_extra as i128);
    let threshold_bps = THRESHOLD_BPS.load(AtomicOrdering::Relaxed) as u128;

    match side {
        0 => {
            // Buy X: Y in, X out.
            let threshold_in = reserve_y.saturating_mul(threshold_bps) / BPS_DENOMINATOR;
            let in1 = input_amount.min(threshold_in);
            let in2 = input_amount.saturating_sub(in1);

            let ask1_q32 = mul_div(anchor_price_q32, BPS_DENOMINATOR + spread1_bps, BPS_DENOMINATOR);
            let ask2_q32 = mul_div(anchor_price_q32, BPS_DENOMINATOR + spread2_bps, BPS_DENOMINATOR);
            if ask1_q32 == 0 || ask2_q32 == 0 {
                return 0;
            }

            let x1 = mul_div(in1, Q32, ask1_q32);
            let x2 = mul_div(in2, Q32, ask2_q32);
            let mut out = x1.saturating_add(x2);
            if out >= reserve_x {
                out = reserve_x.saturating_sub(1);
            }
            out as u64
        }
        1 => {
            // Sell X: X in, Y out.
            let threshold_in = reserve_x.saturating_mul(threshold_bps) / BPS_DENOMINATOR;
            let in1 = input_amount.min(threshold_in);
            let in2 = input_amount.saturating_sub(in1);

            let bid1_factor = BPS_DENOMINATOR.saturating_sub(spread1_bps);
            let bid2_factor = BPS_DENOMINATOR.saturating_sub(spread2_bps);
            let bid1_q32 = mul_div(anchor_price_q32, bid1_factor, BPS_DENOMINATOR);
            let bid2_q32 = mul_div(anchor_price_q32, bid2_factor, BPS_DENOMINATOR);

            let y1 = mul_div(in1, bid1_q32, Q32);
            let y2 = mul_div(in2, bid2_q32, Q32);
            let mut out = y1.saturating_add(y2);
            if out >= reserve_y {
                out = reserve_y.saturating_sub(1);
            }
            out as u64
        }
        _ => 0,
    }
}

fn dealer_after_swap(data: &[u8], storage: &mut [u8]) {
    if data.len() < 42 || storage.len() < 24 {
        return;
    }

    let side = data[1];
    let input_amount = u64::from_le_bytes(data[2..10].try_into().unwrap());
    let output_amount = u64::from_le_bytes(data[10..18].try_into().unwrap());
    let reserve_x = u64::from_le_bytes(data[18..26].try_into().unwrap());
    let reserve_y = u64::from_le_bytes(data[26..34].try_into().unwrap());
    let step = u64::from_le_bytes(data[34..42].try_into().unwrap());
    if reserve_x == 0 || reserve_y == 0 || input_amount == 0 || output_amount == 0 {
        return;
    }

    let trade_price_q32 = match side {
        0 => mul_div(input_amount as u128, Q32, output_amount as u128) as u64,
        1 => mul_div(output_amount as u128, Q32, input_amount as u128) as u64,
        _ => return,
    };
    if trade_price_q32 == 0 {
        return;
    }

    let last_step = read_u64(storage, 0);
    let anchor_price_q32 = read_u64(storage, 8);
    let prev_ewma_q32 = read_u64(storage, 16) as u128;

    if anchor_price_q32 == 0 {
        write_u64(storage, 0, step);
        write_u64(storage, 8, trade_price_q32);
        return;
    }

    if step != last_step {
        let diff = trade_price_q32.abs_diff(anchor_price_q32) as u128;
        let ret_q32 = mul_div(diff, Q32, anchor_price_q32 as u128);
        let next_ewma_q32 = (prev_ewma_q32.saturating_mul(EWMA_ALPHA_NUM) / EWMA_ALPHA_DEN)
            .saturating_add(ret_q32 / EWMA_ALPHA_DEN);
        write_u64(storage, 0, step);
        write_u64(storage, 8, trade_price_q32);
        write_u64(storage, 16, next_ewma_q32 as u64);
    }
}

fn set_params(params: Params) {
    BASE_SPREAD_BPS.store(params.base_spread_bps as u64, AtomicOrdering::Relaxed);
    VOL_SENSITIVITY.store(params.vol_sensitivity as u64, AtomicOrdering::Relaxed);
    TAIL_EXTRA_BPS.store(params.tail_extra_bps as u64, AtomicOrdering::Relaxed);
    THRESHOLD_BPS.store(params.threshold_bps as u64, AtomicOrdering::Relaxed);
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
        dealer_swap,
        Some(dealer_after_swap),
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
        dealer_swap,
        Some(dealer_after_swap),
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
            "#{:02} combined={:8.2} train={:8.2} holdout={:8.2} | base={} vol_k={} tail_extra={} threshold_bps={}",
            idx + 1,
            result.combined_avg_edge,
            result.train_avg_edge,
            result.holdout_avg_edge,
            result.params.base_spread_bps,
            result.params.vol_sensitivity,
            result.params.tail_extra_bps,
            result.params.threshold_bps,
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
    let quick_sims = env_parse("SWEEP_SIMS", 24u32);
    let quick_steps = env_parse("SWEEP_STEPS", 2_000u32);
    let workers = Some(env_parse("SWEEP_WORKERS", 8usize));
    let train_seed_start = env_parse("SWEEP_TRAIN_START", 0u64);
    let holdout_seed_start = env_parse("SWEEP_HOLDOUT_START", 100_000u64);

    println!(
        "Running dealer sweep: sims={} steps={} workers={:?} train_start={} holdout_start={}",
        quick_sims, quick_steps, workers, train_seed_start, holdout_seed_start,
    );

    let base_candidates = [20u16, 30, 40, 50, 60, 80];
    let vol_candidates = [0u16, 4_000, 8_000, 12_000, 16_000];
    let tail_candidates = [100u16, 200, 300, 500, 800, 1_200];
    let threshold_candidates = [10u16, 20, 40, 60, 100];

    let mut quick_results = Vec::new();
    for base_spread_bps in base_candidates {
        for vol_sensitivity in vol_candidates {
            for tail_extra_bps in tail_candidates {
                for threshold_bps in threshold_candidates {
                    quick_results.push(evaluate_params(
                        Params {
                            base_spread_bps,
                            vol_sensitivity,
                            tail_extra_bps,
                            threshold_bps,
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
    let top_quick = top_k(quick_results, 15);
    print_results("Top dealer candidates (quick)", &top_quick);

    let deep_sims = env_parse("DEEP_SIMS", 120u32);
    let deep_steps = env_parse("DEEP_STEPS", 10_000u32);
    println!(
        "\nRunning deep eval on top 8: sims={} steps={}",
        deep_sims, deep_steps
    );

    let mut deep_results = Vec::new();
    for best in top_quick.iter().take(8) {
        deep_results.push(evaluate_params(
            best.params,
            deep_sims,
            deep_steps,
            workers,
            train_seed_start,
            holdout_seed_start,
        )?);
    }
    let top_deep = top_k(deep_results, 8);
    print_results("Top dealer candidates (deep)", &top_deep);

    Ok(())
}
