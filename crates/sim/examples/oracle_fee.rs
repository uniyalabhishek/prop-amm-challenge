use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_shared::config::{HyperparameterVariance, SimulationConfig};
use prop_amm_sim::engine;
use rayon::prelude::*;

/// Swap function that reads fee_bps from storage[0..2]
fn storage_fee_swap(data: &[u8]) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as u128;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as u128;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as u128;
    if rx == 0 || ry == 0 { return 0; }
    
    let fee_bps: u128 = if data.len() >= 27 {
        let raw = u16::from_le_bytes([data[25], data[26]]);
        if raw == 0 { 65 } else { raw as u128 }
    } else {
        65
    };
    
    let gamma_num = 10_000u128.saturating_sub(fee_bps);
    let k = rx * ry;
    match side {
        0 => {
            let net = input * gamma_num / 10_000;
            let new_ry = ry + net;
            rx.saturating_sub((k + new_ry - 1) / new_ry) as u64
        }
        1 => {
            let net = input * gamma_num / 10_000;
            let new_rx = rx + net;
            ry.saturating_sub((k + new_rx - 1) / new_rx) as u64
        }
        _ => 0,
    }
}

/// Run a single simulation with a specific fee for the submission
fn run_sim_with_fee(config: &SimulationConfig, fee_bps: u16) -> f64 {
    use prop_amm_sim::amm::BpfAmm;
    use prop_amm_sim::arbitrageur::Arbitrageur;
    use prop_amm_sim::price_process::GBMPriceProcess;
    use prop_amm_sim::retail::RetailTrader;
    use prop_amm_sim::router::OrderRouter;
    
    let mut amm_sub = BpfAmm::new_native(
        storage_fee_swap,
        None,
        config.initial_x,
        config.initial_y,
        "submission".to_string(),
    );
    amm_sub.set_initial_storage(&fee_bps.to_le_bytes());
    
    let norm_x = config.initial_x * config.norm_liquidity_mult;
    let norm_y = config.initial_y * config.norm_liquidity_mult;
    let mut amm_norm = BpfAmm::new_native(
        normalizer_swap,
        Some(normalizer_after_swap),
        norm_x,
        norm_y,
        "normalizer".to_string(),
    );
    amm_norm.set_initial_storage(&config.norm_fee_bps.to_le_bytes());
    
    let mut price = GBMPriceProcess::new(
        config.initial_price,
        config.gbm_mu,
        config.gbm_sigma,
        config.gbm_dt,
        config.seed,
    );
    let mut retail = RetailTrader::new(
        config.retail_arrival_rate,
        config.retail_mean_size,
        config.retail_size_sigma,
        config.retail_buy_prob,
        config.seed.wrapping_add(1),
    );
    let mut arb = Arbitrageur::new(
        config.min_arb_profit,
        config.retail_mean_size,
        config.retail_size_sigma,
        config.seed.wrapping_add(2),
    );
    let router = OrderRouter::new();
    
    let mut submission_edge = 0.0_f64;
    
    for step in 0..config.n_steps {
        amm_sub.set_current_step(step as u64);
        amm_norm.set_current_step(step as u64);
        let fair_price = price.step();
        
        if let Some(result) = arb.execute_arb(&mut amm_sub, fair_price) {
            submission_edge += result.edge;
        }
        arb.execute_arb(&mut amm_norm, fair_price);
        
        let orders = retail.generate_orders();
        for order in &orders {
            let trades = router.route_order(order, &mut amm_sub, &mut amm_norm, fair_price);
            for trade in trades {
                if trade.is_submission {
                    let trade_edge = if trade.amm_buys_x {
                        trade.amount_x * fair_price - trade.amount_y
                    } else {
                        trade.amount_y - trade.amount_x * fair_price
                    };
                    submission_edge += trade_edge;
                }
            }
        }
    }
    
    submission_edge
}

fn main() {
    let variance = HyperparameterVariance::default();
    let base = SimulationConfig::default();
    let n_sims = 1000u32;
    
    // Compute sigma for each seed
    let configs: Vec<SimulationConfig> = (0..n_sims).map(|i| variance.apply(&base, i as u64)).collect();
    
    // Print sigma distribution
    let mut sigmas: Vec<(usize, f64)> = configs.iter().enumerate().map(|(i, c)| (i, c.gbm_sigma)).collect();
    sigmas.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    
    println!("=== Sigma Distribution ===");
    println!("p5={:.5} p25={:.5} p50={:.5} p75={:.5} p95={:.5}",
        sigmas[50].1, sigmas[250].1, sigmas[500].1, sigmas[750].1, sigmas[950].1);
    
    // Print norm_fee distribution  
    let mut nfees: Vec<u16> = configs.iter().map(|c| c.norm_fee_bps).collect();
    nfees.sort();
    println!("\nNorm fee: p5={} p25={} p50={} p75={} p95={}",
        nfees[50], nfees[250], nfees[500], nfees[750], nfees[950]);
    
    let mut nliqs: Vec<f64> = configs.iter().map(|c| c.norm_liquidity_mult).collect();
    nliqs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("Norm liq: p5={:.2} p25={:.2} p50={:.2} p75={:.2} p95={:.2}",
        nliqs[50], nliqs[250], nliqs[500], nliqs[750], nliqs[950]);
    
    // Group by sigma buckets
    let bucket_ranges: Vec<(f64, f64)> = vec![
        (0.0, 0.001), (0.001, 0.002), (0.002, 0.003),
        (0.003, 0.004), (0.004, 0.005), (0.005, 0.006), (0.006, 0.007),
    ];
    
    let fee_options: Vec<u16> = vec![20, 30, 40, 50, 55, 60, 65, 70, 75, 80, 90, 100, 120, 150, 200];
    
    println!("\n=== Oracle Fee by Sigma Bucket ===");
    let mut oracle_total = 0.0f64;
    let mut oracle_count = 0usize;
    
    for (lo, hi) in &bucket_ranges {
        let bucket_indices: Vec<usize> = configs.iter().enumerate()
            .filter(|(_, c)| c.gbm_sigma >= *lo && c.gbm_sigma < *hi)
            .map(|(i, _)| i)
            .collect();
        
        if bucket_indices.is_empty() { continue; }
        
        let mut best_fee = 65u16;
        let mut best_total = f64::NEG_INFINITY;
        
        for &fee in &fee_options {
            let total: f64 = bucket_indices.par_iter().map(|&idx| {
                run_sim_with_fee(&configs[idx], fee)
            }).sum();
            
            if total > best_total {
                best_total = total;
                best_fee = fee;
            }
        }
        
        oracle_total += best_total;
        oracle_count += bucket_indices.len();
        
        println!("  σ [{:.3}%, {:.3}%): {} sims, best_fee={}bp, avg_edge={:.2}",
            lo * 100.0, hi * 100.0, bucket_indices.len(), best_fee,
            best_total / bucket_indices.len() as f64);
    }
    
    println!("\n=== Oracle Score ===");
    println!("  Oracle avg edge: {:.2}", oracle_total / oracle_count as f64);
    
    // Constant 65bp baseline
    let baseline: f64 = configs.par_iter().map(|config| {
        run_sim_with_fee(config, 65)
    }).sum();
    println!("  Constant 65bp avg edge: {:.2}", baseline / n_sims as f64);
    
    // Also try finer sigma-based fee with more granularity
    println!("\n=== Per-Sim Oracle (brute force best fee per seed) on 100 sims ===");
    let mut per_sim_total = 0.0f64;
    let per_sim_count = 100;
    
    for i in 0..per_sim_count {
        let config = &configs[i];
        let mut best_edge = f64::NEG_INFINITY;
        let mut best_fee = 65u16;
        
        for &fee in &fee_options {
            let edge = run_sim_with_fee(config, fee);
            if edge > best_edge {
                best_edge = edge;
                best_fee = fee;
            }
        }
        
        per_sim_total += best_edge;
        if i < 10 {
            println!("  seed={}: σ={:.5}, norm_fee={}bp, norm_liq={:.2}, best_fee={}bp, edge={:.2}",
                i, config.gbm_sigma, config.norm_fee_bps, config.norm_liquidity_mult, best_fee, best_edge);
        }
    }
    
    println!("  Per-sim oracle avg edge (first 100): {:.2}", per_sim_total / per_sim_count as f64);
}
