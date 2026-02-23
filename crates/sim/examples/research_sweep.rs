use std::cmp::Ordering;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use anyhow::Result;
use prop_amm_shared::normalizer::{
    after_swap as normalizer_after_swap, compute_swap as normalizer_swap,
};
use prop_amm_sim::runner;

const BPS_DENOMINATOR: u128 = 10_000;
const MAX_FEE_BPS: u128 = BPS_DENOMINATOR - 1;

static FEE_LOW_BPS: AtomicU64 = AtomicU64::new(50);
static FEE_HIGH_BPS: AtomicU64 = AtomicU64::new(50);
static THRESHOLD_BPS: AtomicU64 = AtomicU64::new(100);

#[derive(Clone, Copy, Debug)]
struct Params {
    fee_low_bps: u16,
    fee_high_bps: u16,
    threshold_bps: u16,
}

#[derive(Clone, Debug)]
struct EvalResult {
    params: Params,
    train_avg_edge: f64,
    holdout_avg_edge: f64,
    combined_avg_edge: f64,
}

fn piecewise_cp_swap(data: &[u8]) -> u64 {
    if data.len() < 25 {
        return 0;
    }

    let side = data[0];
    let input_amount = u64::from_le_bytes(data[1..9].try_into().unwrap()) as u128;
    let reserve_x = u64::from_le_bytes(data[9..17].try_into().unwrap()) as u128;
    let reserve_y = u64::from_le_bytes(data[17..25].try_into().unwrap()) as u128;

    if reserve_x == 0 || reserve_y == 0 || input_amount == 0 {
        return 0;
    }

    let fee_low_bps = (FEE_LOW_BPS.load(AtomicOrdering::Relaxed) as u128).min(MAX_FEE_BPS);
    let fee_high_bps = (FEE_HIGH_BPS.load(AtomicOrdering::Relaxed) as u128).min(MAX_FEE_BPS);
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

fn set_params(params: Params) {
    FEE_LOW_BPS.store(params.fee_low_bps as u64, AtomicOrdering::Relaxed);
    FEE_HIGH_BPS.store(params.fee_high_bps as u64, AtomicOrdering::Relaxed);
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
        piecewise_cp_swap,
        None,
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
        piecewise_cp_swap,
        None,
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
            "#{:02} combined={:8.2} train={:8.2} holdout={:8.2} | fee_low={} fee_high={} threshold_bps={}",
            idx + 1,
            result.combined_avg_edge,
            result.train_avg_edge,
            result.holdout_avg_edge,
            result.params.fee_low_bps,
            result.params.fee_high_bps,
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
    let quick_sims = env_parse("SWEEP_SIMS", 48u32);
    let quick_steps = env_parse("SWEEP_STEPS", 3_000u32);
    let workers = Some(env_parse("SWEEP_WORKERS", 8usize));
    let train_seed_start = env_parse("SWEEP_TRAIN_START", 0u64);
    let holdout_seed_start = env_parse("SWEEP_HOLDOUT_START", 100_000u64);

    println!(
        "Running quick sweep: sims={} steps={} workers={:?} train_start={} holdout_start={}",
        quick_sims, quick_steps, workers, train_seed_start, holdout_seed_start,
    );

    let constant_fee_candidates = [20u16, 30, 40, 50, 60, 70, 80, 100, 120];
    let mut constant_results = Vec::new();
    for fee in constant_fee_candidates {
        constant_results.push(evaluate_params(
            Params {
                fee_low_bps: fee,
                fee_high_bps: fee,
                threshold_bps: 100,
            },
            quick_sims,
            quick_steps,
            workers,
            train_seed_start,
            holdout_seed_start,
        )?);
    }

    let top_constant = top_k(constant_results.clone(), 4);
    print_results("Top constant-fee candidates", &top_constant);

    let high_fee_offsets = [0u16, 20, 40, 80];
    let threshold_candidates = [5u16, 10, 20, 40, 80, 160];

    let mut piecewise_results = Vec::new();
    for base in &top_constant {
        let low_fee = base.params.fee_low_bps;
        for offset in high_fee_offsets {
            let high_fee = low_fee.saturating_add(offset).min(500);
            for threshold_bps in threshold_candidates {
                piecewise_results.push(evaluate_params(
                    Params {
                        fee_low_bps: low_fee,
                        fee_high_bps: high_fee,
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

    let top_piecewise = top_k(piecewise_results, 15);
    print_results("Top piecewise candidates (quick sweep)", &top_piecewise);

    let deep_sims = env_parse("DEEP_SIMS", 200u32);
    let deep_steps = env_parse("DEEP_STEPS", 10_000u32);
    println!(
        "\nRunning deep eval on top 5: sims={} steps={}",
        deep_sims, deep_steps
    );

    let mut deep_results = Vec::new();
    for quick_best in top_piecewise.iter().take(5) {
        deep_results.push(evaluate_params(
            quick_best.params,
            deep_sims,
            deep_steps,
            workers,
            train_seed_start,
            holdout_seed_start,
        )?);
    }

    let top_deep = top_k(deep_results, 5);
    print_results("Top candidates after deep eval", &top_deep);

    Ok(())
}
