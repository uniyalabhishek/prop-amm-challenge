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

fn exp_swap_fee(data: &[u8], fee_bps: f64) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return 0; }
    let gamma = (10000.0 - fee_bps) / 10000.0;
    let alpha = 0.30;
    let (ri, ro) = match side { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    let u = gamma * input / (ri * alpha);
    let output = alpha * ro * (1.0 - (-u).exp());
    if output <= 0.0 || !output.is_finite() { return 0; }
    let s = (output.min(ro * 0.999) * NANO).floor();
    if s <= 0.0 || s >= u64::MAX as f64 { 0 } else { s as u64 }
}

fn storage_fee_swap(data: &[u8]) -> u64 {
    if data.len() < 27 { return exp_swap_fee(data, 55.0); }
    let fee = u16::from_le_bytes([data[25], data[26]]) as f64;
    exp_swap_fee(data, if fee == 0.0 { 55.0 } else { fee })
}

fn run_sim(config: &SimulationConfig, fee_low: u16, fee_high: u16, sigma_thresh_bps: f64) -> f64 {
    let mut amm_sub = BpfAmm::new_native(
        storage_fee_swap, None, config.initial_x, config.initial_y, "submission".to_string());
    amm_sub.set_initial_storage(&fee_low.to_le_bytes());
    
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
    
    // Decide fee based on actual σ (oracle-like, but using step function)
    let sigma_bps = config.gbm_sigma * 10000.0;
    let fee = if sigma_bps > sigma_thresh_bps { fee_high } else { fee_low };
    amm_sub.set_initial_storage(&fee.to_le_bytes());
    
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
    let configs: Vec<SimulationConfig> = (0..1000u32).map(|i| var.apply(&base, i as u64)).collect();
    
    // Test different step-function configurations
    println!("=== Step-function fee oracle (perfect σ knowledge) ===");
    
    let combos: Vec<(u16, u16, f64, &str)> = vec![
        (55, 55, 50.0, "constant 55bp"),
        (55, 100, 40.0, "55/100 @40"),
        (55, 100, 45.0, "55/100 @45"),
        (55, 100, 50.0, "55/100 @50"),
        (55, 120, 45.0, "55/120 @45"),
        (55, 120, 50.0, "55/120 @50"),
        (55, 130, 50.0, "55/130 @50"),
        (55, 130, 55.0, "55/130 @55"),
        (55, 150, 45.0, "55/150 @45"),
        (55, 150, 50.0, "55/150 @50"),
        (55, 150, 55.0, "55/150 @55"),
        (55, 200, 50.0, "55/200 @50"),
        (50, 120, 45.0, "50/120 @45"),
        (50, 130, 50.0, "50/130 @50"),
        (50, 150, 50.0, "50/150 @50"),
        (60, 130, 50.0, "60/130 @50"),
        (60, 150, 50.0, "60/150 @50"),
    ];
    
    for (fl, fh, thresh, name) in &combos {
        let total: f64 = configs.par_iter().map(|c| run_sim(c, *fl, *fh, *thresh)).sum();
        println!("{:>20}: avg={:.2}", name, total / 1000.0);
    }
}
