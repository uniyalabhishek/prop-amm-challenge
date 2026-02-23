use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_shared::config::{HyperparameterVariance, SimulationConfig};
use prop_amm_sim::amm::BpfAmm;
use prop_amm_sim::arbitrageur::Arbitrageur;
use prop_amm_sim::price_process::GBMPriceProcess;
use prop_amm_sim::retail::RetailTrader;
use prop_amm_sim::router::OrderRouter;
use rayon::prelude::*;

const NANO: f64 = 1_000_000_000.0;

fn exp_swap_params(data: &[u8], fee_bps: f64, alpha: f64) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return 0; }
    let gamma = (10000.0 - fee_bps) / 10000.0;
    let (ri, ro) = match side { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    let u = gamma * input / (ri * alpha);
    let output = alpha * ro * (1.0 - (-u).exp());
    if output <= 0.0 || !output.is_finite() { return 0; }
    let scaled = (output.min(ro * 0.999) * NANO).floor();
    if scaled <= 0.0 || scaled >= u64::MAX as f64 { 0 } else { scaled as u64 }
}

// Storage-based swap for run_sim
fn storage_swap(data: &[u8]) -> u64 {
    if data.len() < 29 { return exp_swap_params(data, 55.0, 0.30); }
    let fee = u16::from_le_bytes([data[25], data[26]]) as f64;
    let alpha_pct = u16::from_le_bytes([data[27], data[28]]) as f64;
    let fee = if fee == 0.0 { 55.0 } else { fee };
    let alpha = if alpha_pct == 0.0 { 0.30 } else { alpha_pct / 100.0 };
    exp_swap_params(data, fee, alpha)
}

fn run_sim(config: &SimulationConfig, fee_bps: u16, alpha_pct: u16) -> f64 {
    let mut amm_sub = BpfAmm::new_native(
        storage_swap, None, config.initial_x, config.initial_y, "submission".to_string());
    let mut s = [0u8; 4];
    s[0..2].copy_from_slice(&fee_bps.to_le_bytes());
    s[2..4].copy_from_slice(&alpha_pct.to_le_bytes());
    amm_sub.set_initial_storage(&s);
    
    let nx = config.initial_x * config.norm_liquidity_mult;
    let ny = config.initial_y * config.norm_liquidity_mult;
    let mut amm_norm = BpfAmm::new_native(
        normalizer_swap, Some(normalizer_after_swap), nx, ny, "normalizer".to_string());
    amm_norm.set_initial_storage(&config.norm_fee_bps.to_le_bytes());
    
    let mut price = GBMPriceProcess::new(
        config.initial_price, config.gbm_mu, config.gbm_sigma, config.gbm_dt, config.seed);
    let mut retail = RetailTrader::new(
        config.retail_arrival_rate, config.retail_mean_size, config.retail_size_sigma,
        config.retail_buy_prob, config.seed.wrapping_add(1));
    let mut arb = Arbitrageur::new(
        config.min_arb_profit, config.retail_mean_size, config.retail_size_sigma,
        config.seed.wrapping_add(2));
    let router = OrderRouter::new();
    let mut edge = 0.0f64;
    for step in 0..config.n_steps {
        amm_sub.set_current_step(step as u64);
        amm_norm.set_current_step(step as u64);
        let fair = price.step();
        if let Some(r) = arb.execute_arb(&mut amm_sub, fair) { edge += r.edge; }
        arb.execute_arb(&mut amm_norm, fair);
        for order in &retail.generate_orders() {
            for trade in router.route_order(order, &mut amm_sub, &mut amm_norm, fair) {
                if trade.is_submission {
                    edge += if trade.amm_buys_x {
                        trade.amount_x * fair - trade.amount_y
                    } else {
                        trade.amount_y - trade.amount_x * fair
                    };
                }
            }
        }
    }
    edge
}

fn main() {
    let variance = HyperparameterVariance::default();
    let base = SimulationConfig::default();
    let configs: Vec<SimulationConfig> = (0..200u32).map(|i| variance.apply(&base, i as u64)).collect();
    
    // Full grid: fee × alpha (reduced grid for speed)
    let fees: Vec<u16> = vec![30, 40, 50, 55, 60, 70, 80, 100, 150, 200];
    let alphas: Vec<u16> = vec![15, 20, 25, 30, 35, 40, 50]; // as pct
    
    println!("Running full oracle on 200 sims with {} fee × {} alpha = {} combinations...",
        fees.len(), alphas.len(), fees.len() * alphas.len());
    
    let per_sim_results: Vec<(usize, u16, u16, f64)> = (0..200usize).into_par_iter().map(|i| {
        let config = &configs[i];
        let mut best_fee = 55u16;
        let mut best_alpha = 30u16;
        let mut best_edge = f64::NEG_INFINITY;
        for &fee in &fees {
            for &alpha in &alphas {
                let edge = run_sim(config, fee, alpha);
                if edge > best_edge {
                    best_edge = edge;
                    best_fee = fee;
                    best_alpha = alpha;
                }
            }
        }
        (i, best_fee, best_alpha, best_edge)
    }).collect();
    
    let oracle_total: f64 = per_sim_results.iter().map(|(_, _, _, e)| e).sum();
    let oracle_avg = oracle_total / 200.0;
    
    println!("\n=== Full Oracle Results (500 sims, {} combos) ===", fees.len() * alphas.len());
    println!("Oracle avg edge: {:.2}", oracle_avg);
    
    // Distribution of best parameters
    let mut fee_counts = std::collections::HashMap::new();
    let mut alpha_counts = std::collections::HashMap::new();
    for (_, f, a, _) in &per_sim_results {
        *fee_counts.entry(*f).or_insert(0u32) += 1;
        *alpha_counts.entry(*a).or_insert(0u32) += 1;
    }
    
    let mut fee_vec: Vec<_> = fee_counts.into_iter().collect();
    fee_vec.sort_by_key(|(f, _)| *f);
    println!("\nBest fee distribution:");
    for (f, c) in &fee_vec { println!("  {}bp: {} sims ({:.1}%)", f, c, *c as f64 / 2.0); }
    
    let mut alpha_vec: Vec<_> = alpha_counts.into_iter().collect();
    alpha_vec.sort_by_key(|(a, _)| *a);
    println!("\nBest alpha distribution:");
    for (a, c) in &alpha_vec { println!("  {:.2}: {} sims ({:.1}%)", *a as f64/100.0, c, *c as f64 / 2.0); }
    
    // Show some example sims
    println!("\nFirst 20 sims:");
    for (i, f, a, e) in per_sim_results.iter().take(20) {
        let c = &configs[*i];
        println!("  seed={}: σ={:.5}, nf={}bp, nl={:.2} → best=({:.2}, {}bp), edge={:.1}",
            i, c.gbm_sigma, c.norm_fee_bps, c.norm_liquidity_mult,
            *a as f64/100.0, f, e);
    }
    
    // Constant baseline
    let baseline: f64 = configs.par_iter().map(|c| run_sim(c, 55, 30)).sum();
    println!("\nConstant exp(0.30, 55bp) avg: {:.2}", baseline / 200.0);
}
