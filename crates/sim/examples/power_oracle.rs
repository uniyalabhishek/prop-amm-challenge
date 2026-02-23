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

/// Power curve: output = ro * (1 - (1 + γ*input/(α*ri))^(-α))
/// Same marginal as CP. Approaches FULL reserves for large input (unlike exp).
/// More concave than CP for α < 1.
fn power_swap_params(data: &[u8], fee_bps: f64, alpha: f64) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return 0; }
    let gamma = (10000.0 - fee_bps) / 10000.0;
    let (ri, ro) = match side { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    let u = gamma * input / (alpha * ri);
    let output = ro * (1.0 - (1.0 + u).powf(-alpha));
    if output <= 0.0 || !output.is_finite() { return 0; }
    let s = (output.min(ro * 0.999) * NANO).floor();
    if s <= 0.0 || s >= u64::MAX as f64 { 0 } else { s as u64 }
}

fn storage_power_swap(data: &[u8]) -> u64 {
    if data.len() < 29 { return power_swap_params(data, 55.0, 0.40); }
    let fee = u16::from_le_bytes([data[25], data[26]]) as f64;
    let alpha_pct = u16::from_le_bytes([data[27], data[28]]) as f64;
    let fee = if fee == 0.0 { 55.0 } else { fee };
    let alpha = if alpha_pct == 0.0 { 0.40 } else { alpha_pct / 100.0 };
    power_swap_params(data, fee, alpha)
}

fn run_sim(config: &SimulationConfig, fee_bps: u16, alpha_pct: u16) -> f64 {
    let mut amm_sub = BpfAmm::new_native(
        storage_power_swap, None, config.initial_x, config.initial_y, "submission".to_string());
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
    let var = HyperparameterVariance::default();
    let base = SimulationConfig::default();
    let configs: Vec<SimulationConfig> = (0..200u32).map(|i| var.apply(&base, i as u64)).collect();

    // Wider grid including high-α (near-CP) options  
    let fees: Vec<u16> = vec![30, 40, 50, 55, 60, 65, 70, 80, 100, 120, 150, 200];
    let alphas: Vec<u16> = vec![15, 20, 25, 30, 35, 40, 50, 60, 70, 80, 90, 100]; // pct, 100=CP

    println!("Power oracle: 200 sims, {} fees × {} alphas = {} combos",
        fees.len(), alphas.len(), fees.len() * alphas.len());

    let results: Vec<(usize, u16, u16, f64)> = (0..200usize).into_par_iter().map(|i| {
        let mut best_fee = 55u16;
        let mut best_alpha = 40u16;
        let mut best_edge = f64::NEG_INFINITY;
        for &fee in &fees {
            for &alpha in &alphas {
                let edge = run_sim(&configs[i], fee, alpha);
                if edge > best_edge { best_edge = edge; best_fee = fee; best_alpha = alpha; }
            }
        }
        (i, best_fee, best_alpha, best_edge)
    }).collect();

    let total: f64 = results.iter().map(|(_, _, _, e)| e).sum();
    println!("Power oracle avg: {:.2}", total / 200.0);

    // Distribution of best alpha
    let mut alpha_counts = std::collections::HashMap::new();
    for (_, _, a, _) in &results {
        *alpha_counts.entry(*a).or_insert(0u32) += 1;
    }
    let mut av: Vec<_> = alpha_counts.into_iter().collect();
    av.sort_by_key(|(a, _)| *a);
    println!("\nBest alpha distribution:");
    for (a, c) in &av { println!("  {:.2}: {} sims", *a as f64/100.0, c); }

    // Show some high-edge sims
    let mut sorted = results.clone();
    sorted.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
    println!("\nTop 10 sims:");
    for (i, f, a, e) in sorted.iter().take(10) {
        let c = &configs[*i];
        println!("  seed={}: σ={:.5}, nf={}bp, nl={:.2} → ({:.2}, {}bp) edge={:.1}",
            i, c.gbm_sigma, c.norm_fee_bps, c.norm_liquidity_mult, *a as f64/100.0, f, e);
    }

    // Baseline with power(0.40, 55bp)
    let bl: f64 = configs.par_iter().map(|c| run_sim(c, 55, 40)).sum();
    println!("\nConstant power(0.40, 55bp): {:.2}", bl / 200.0);

    // Also exp oracle for comparison
    let bl2: f64 = configs.par_iter().map(|c| run_sim(c, 55, 30)).sum();
    println!("Constant power(0.30, 55bp): {:.2}", bl2 / 200.0);
}
