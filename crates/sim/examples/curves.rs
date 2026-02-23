use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_sim::runner;
use std::time::Instant;

const NANO: f64 = 1_000_000_000.0;

fn parse_data(data: &[u8]) -> Option<(u8, f64, f64, f64)> {
    if data.len() < 25 { return None; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return None; }
    Some((side, input, rx, ry))
}

fn to_nano_u64(val: f64) -> u64 {
    if val <= 0.0 || !val.is_finite() { return 0; }
    let scaled = val * NANO;
    if scaled >= u64::MAX as f64 { u64::MAX } else { scaled as u64 }
}

// Standard constant product
fn cp_swap(data: &[u8], fee_bps: f64) -> u64 {
    let (side, input, rx, ry) = match parse_data(data) {
        Some(v) => v,
        None => return 0,
    };
    let gamma = (10000.0 - fee_bps) / 10000.0;
    let output = match side {
        0 => {
            // Buy X: input Y, output X
            let net = input * gamma;
            rx * net / (ry + net)
        }
        1 => {
            // Sell X: input X, output Y
            let net = input * gamma;
            ry * net / (rx + net)
        }
        _ => return 0,
    };
    if output <= 0.0 || !output.is_finite() { return 0; }
    let max_out = match side { 0 => rx, 1 => ry, _ => 0.0 };
    to_nano_u64(output.min(max_out * 0.999))
}

// Exponential curve: output = alpha*reserve_out * (1 - exp(-gamma*input / (alpha*reserve_in)))
// Same marginal as CP at input=0, but more concave for larger inputs
// alpha controls how quickly it saturates (lower alpha = more concave, less arb loss)
fn exp_swap(data: &[u8], fee_bps: f64, alpha: f64) -> u64 {
    let (side, input, rx, ry) = match parse_data(data) {
        Some(v) => v,
        None => return 0,
    };
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
    to_nano_u64(output.min(max_out * 0.999))
}

// Power curve: output = alpha*reserve_out * ((1 + gamma*input/(alpha*reserve_in))^p - 1) / ((1+x)^p -1 where x = gamma*input/(alpha*reserve_in))
// Wait, that grows without bound. Let me use a different formulation.
// output = reserve_out * (1 - (1 + gamma*input/reserve_in)^(-alpha))
// For alpha=1: standard CP, for alpha>1: steeper (more concave), for alpha<1: flatter
// Marginal at 0: alpha * gamma * reserve_out / reserve_in
fn power_swap(data: &[u8], fee_bps: f64, alpha: f64) -> u64 {
    let (side, input, rx, ry) = match parse_data(data) {
        Some(v) => v,
        None => return 0,
    };
    let gamma = (10000.0 - fee_bps) / 10000.0;
    
    // Adjust fee to maintain same marginal as standard CP
    // Standard CP marginal: gamma * rx/ry
    // Power marginal: alpha * gamma * rx/ry
    // To match: use gamma_adj = gamma / alpha
    let gamma_adj = gamma / alpha;
    
    let output = match side {
        0 => {
            let u = gamma_adj * input / ry;
            rx * (1.0 - (1.0 + u).powf(-alpha))
        }
        1 => {
            let u = gamma_adj * input / rx;
            ry * (1.0 - (1.0 + u).powf(-alpha))
        }
        _ => return 0,
    };
    if output <= 0.0 || !output.is_finite() { return 0; }
    let max_out = match side { 0 => rx, 1 => ry, _ => 0.0 };
    to_nano_u64(output.min(max_out * 0.999))
}

// Define concrete functions for each configuration
fn cp_50(d: &[u8]) -> u64 { cp_swap(d, 50.0) }
fn cp_60(d: &[u8]) -> u64 { cp_swap(d, 60.0) }
fn cp_65(d: &[u8]) -> u64 { cp_swap(d, 65.0) }
fn cp_70(d: &[u8]) -> u64 { cp_swap(d, 70.0) }
fn cp_80(d: &[u8]) -> u64 { cp_swap(d, 80.0) }

// Exponential curves with various alpha and fee
fn exp_a05_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.5) }
fn exp_a05_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.5) }
fn exp_a05_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.5) }
fn exp_a05_f70(d: &[u8]) -> u64 { exp_swap(d, 70.0, 0.5) }
fn exp_a05_f80(d: &[u8]) -> u64 { exp_swap(d, 80.0, 0.5) }

fn exp_a03_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.3) }
fn exp_a03_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.3) }
fn exp_a03_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.3) }
fn exp_a03_f70(d: &[u8]) -> u64 { exp_swap(d, 70.0, 0.3) }
fn exp_a03_f80(d: &[u8]) -> u64 { exp_swap(d, 80.0, 0.3) }

fn exp_a01_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.1) }
fn exp_a01_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.1) }
fn exp_a01_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.1) }
fn exp_a01_f70(d: &[u8]) -> u64 { exp_swap(d, 70.0, 0.1) }

fn exp_a08_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.8) }
fn exp_a08_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.8) }
fn exp_a08_f70(d: &[u8]) -> u64 { exp_swap(d, 70.0, 0.8) }

fn exp_a10_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 1.0) }

// Power curves 
fn pow_a15_f65(d: &[u8]) -> u64 { power_swap(d, 65.0, 1.5) }
fn pow_a20_f65(d: &[u8]) -> u64 { power_swap(d, 65.0, 2.0) }
fn pow_a30_f65(d: &[u8]) -> u64 { power_swap(d, 65.0, 3.0) }
fn pow_a15_f50(d: &[u8]) -> u64 { power_swap(d, 50.0, 1.5) }
fn pow_a20_f50(d: &[u8]) -> u64 { power_swap(d, 50.0, 2.0) }
fn pow_a15_f80(d: &[u8]) -> u64 { power_swap(d, 80.0, 1.5) }
fn pow_a20_f80(d: &[u8]) -> u64 { power_swap(d, 80.0, 2.0) }
fn pow_a50_f65(d: &[u8]) -> u64 { power_swap(d, 65.0, 5.0) }
fn pow_a50_f50(d: &[u8]) -> u64 { power_swap(d, 50.0, 5.0) }
fn pow_a100_f65(d: &[u8]) -> u64 { power_swap(d, 65.0, 10.0) }

fn run_test(name: &str, swap_fn: fn(&[u8]) -> u64, n_sims: u32) {
    let start = Instant::now();
    let result = runner::run_default_batch_native(
        swap_fn,
        None,
        normalizer_swap,
        Some(normalizer_after_swap),
        n_sims,
        10_000,
        None,
    ).unwrap();
    let elapsed = start.elapsed();
    println!("{:>30}: avg_edge={:>8.2}  time={:.1}s", 
        name, result.avg_edge(), elapsed.as_secs_f64());
}

fn main() {
    let n = 1000u32;
    
    println!("=== CP Baselines (f64) ===");
    run_test("CP 50bp (f64)", cp_50, n);
    run_test("CP 60bp (f64)", cp_60, n);
    run_test("CP 65bp (f64)", cp_65, n);
    run_test("CP 70bp (f64)", cp_70, n);
    run_test("CP 80bp (f64)", cp_80, n);
    
    println!("\n=== Exponential α=1.0 (≈CP) ===");
    run_test("Exp α=1.0 65bp", exp_a10_f65, n);
    
    println!("\n=== Exponential α=0.8 ===");
    run_test("Exp α=0.8 60bp", exp_a08_f60, n);
    run_test("Exp α=0.8 65bp", exp_a08_f65, n);
    run_test("Exp α=0.8 70bp", exp_a08_f70, n);
    
    println!("\n=== Exponential α=0.5 ===");
    run_test("Exp α=0.5 50bp", exp_a05_f50, n);
    run_test("Exp α=0.5 60bp", exp_a05_f60, n);
    run_test("Exp α=0.5 65bp", exp_a05_f65, n);
    run_test("Exp α=0.5 70bp", exp_a05_f70, n);
    run_test("Exp α=0.5 80bp", exp_a05_f80, n);
    
    println!("\n=== Exponential α=0.3 ===");
    run_test("Exp α=0.3 50bp", exp_a03_f50, n);
    run_test("Exp α=0.3 60bp", exp_a03_f60, n);
    run_test("Exp α=0.3 65bp", exp_a03_f65, n);
    run_test("Exp α=0.3 70bp", exp_a03_f70, n);
    run_test("Exp α=0.3 80bp", exp_a03_f80, n);
    
    println!("\n=== Exponential α=0.1 ===");
    run_test("Exp α=0.1 50bp", exp_a01_f50, n);
    run_test("Exp α=0.1 60bp", exp_a01_f60, n);
    run_test("Exp α=0.1 65bp", exp_a01_f65, n);
    run_test("Exp α=0.1 70bp", exp_a01_f70, n);
    
    println!("\n=== Power α>1 (more concave than CP) ===");
    run_test("Pow α=1.5 50bp", pow_a15_f50, n);
    run_test("Pow α=1.5 65bp", pow_a15_f65, n);
    run_test("Pow α=1.5 80bp", pow_a15_f80, n);
    run_test("Pow α=2.0 50bp", pow_a20_f50, n);
    run_test("Pow α=2.0 65bp", pow_a20_f65, n);
    run_test("Pow α=2.0 80bp", pow_a20_f80, n);
    run_test("Pow α=3.0 65bp", pow_a30_f65, n);
    run_test("Pow α=5.0 65bp", pow_a50_f65, n);
    run_test("Pow α=5.0 50bp", pow_a50_f50, n);
    run_test("Pow α=10 65bp", pow_a100_f65, n);
}
