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

fn exp_swap_storage_fee(data: &[u8]) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return 0; }
    
    // Read fee from storage[0..2] and alpha from storage[2..4]
    let fee_bps: f64 = if data.len() >= 27 {
        let raw = u16::from_le_bytes([data[25], data[26]]);
        if raw == 0 { 55.0 } else { raw as f64 }
    } else {
        55.0
    };
    let alpha: f64 = if data.len() >= 29 {
        let raw = u16::from_le_bytes([data[27], data[28]]);
        if raw == 0 { 30.0 } else { raw as f64 }
    } else {
        30.0
    };
    let alpha = alpha / 100.0; // stored as percentage (30 = 0.30)
    
    let gamma = (10000.0 - fee_bps) / 10000.0;
    let output = match side {
        0 => {
            let u = gamma * input / (ry * alpha);
            alpha * rx * (1.0 - (-u).exp())
        }
        1 => {
            let u = gamma * input / (rx * alpha);
            alpha * ry * (1.0 - (-u).exp())
        }
        _ => return 0,
    };
    if output <= 0.0 || !output.is_finite() { return 0; }
    let max_out = match side { 0 => rx, 1 => ry, _ => 0.0 };
    let scaled = (output.min(max_out * 0.999) * NANO).floor();
    if scaled <= 0.0 || scaled >= u64::MAX as f64 { return 0; }
    scaled as u64
}

fn run_sim_with_params(config: &SimulationConfig, fee_bps: u16, alpha_pct: u16) -> f64 {
    let mut amm_sub = BpfAmm::new_native(
        exp_swap_storage_fee, None,
        config.initial_x, config.initial_y, "submission".to_string(),
    );
    let mut storage = [0u8; 4];
    storage[0..2].copy_from_slice(&fee_bps.to_le_bytes());
    storage[2..4].copy_from_slice(&alpha_pct.to_le_bytes());
    amm_sub.set_initial_storage(&storage);
    
    let norm_x = config.initial_x * config.norm_liquidity_mult;
    let norm_y = config.initial_y * config.norm_liquidity_mult;
    let mut amm_norm = BpfAmm::new_native(
        normalizer_swap, Some(normalizer_after_swap),
        norm_x, norm_y, "normalizer".to_string(),
    );
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
    
    let mut edge = 0.0_f64;
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
    let configs: Vec<SimulationConfig> = (0..1000u32).map(|i| variance.apply(&base, i as u64)).collect();
    
    // Fee options to test per bucket
    let fee_options: Vec<u16> = vec![30, 40, 45, 50, 55, 60, 65, 70, 80, 90, 100, 120, 150, 200];
    let alpha_options: Vec<u16> = vec![20, 25, 30, 35, 40]; // as percentages
    
    let bucket_ranges: Vec<(f64, f64)> = vec![
        (0.0, 0.001), (0.001, 0.002), (0.002, 0.003),
        (0.003, 0.004), (0.004, 0.005), (0.005, 0.006), (0.006, 0.007),
    ];
    
    println!("=== Oracle: Exp curve with per-bucket optimal (fee, alpha) ===");
    let mut oracle_total = 0.0f64;
    let mut oracle_count = 0usize;
    
    for (lo, hi) in &bucket_ranges {
        let bucket_idx: Vec<usize> = configs.iter().enumerate()
            .filter(|(_, c)| c.gbm_sigma >= *lo && c.gbm_sigma < *hi)
            .map(|(i, _)| i).collect();
        
        if bucket_idx.is_empty() { continue; }
        
        let mut best_fee = 55u16;
        let mut best_alpha = 30u16;
        let mut best_total = f64::NEG_INFINITY;
        
        for &fee in &fee_options {
            for &alpha in &alpha_options {
                let total: f64 = bucket_idx.par_iter().map(|&i| {
                    run_sim_with_params(&configs[i], fee, alpha)
                }).sum();
                if total > best_total {
                    best_total = total;
                    best_fee = fee;
                    best_alpha = alpha;
                }
            }
        }
        
        oracle_total += best_total;
        oracle_count += bucket_idx.len();
        
        println!("  σ [{:.3}%, {:.3}%): {} sims, best_fee={}bp, best_α={:.2}, avg_edge={:.2}",
            lo*100.0, hi*100.0, bucket_idx.len(), best_fee, best_alpha as f64/100.0,
            best_total / bucket_idx.len() as f64);
    }
    
    println!("\nOracle avg edge: {:.2}", oracle_total / oracle_count as f64);
    
    // Constant baseline
    let baseline: f64 = configs.par_iter().map(|c| run_sim_with_params(c, 55, 30)).sum();
    println!("Constant exp(0.3,55bp) avg: {:.2}", baseline / 1000.0);
}
