use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_sim::runner;
use std::time::Instant;

const NANO: f64 = 1_000_000_000.0;

fn exp_swap_core(data: &[u8], fee_bps: f64, alpha: f64) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return 0; }
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
    if scaled <= 0.0 || scaled >= u64::MAX as f64 { 0 } else { scaled as u64 }
}

// Define functions for a fine grid: alpha x fee
// Alpha: 0.22, 0.25, 0.27, 0.30, 0.32, 0.35
// Fee: 40, 45, 50, 52, 55, 57, 60
fn e22_40(d: &[u8]) -> u64 { exp_swap_core(d, 40.0, 0.22) }
fn e22_45(d: &[u8]) -> u64 { exp_swap_core(d, 45.0, 0.22) }
fn e22_50(d: &[u8]) -> u64 { exp_swap_core(d, 50.0, 0.22) }
fn e22_55(d: &[u8]) -> u64 { exp_swap_core(d, 55.0, 0.22) }

fn e25_40(d: &[u8]) -> u64 { exp_swap_core(d, 40.0, 0.25) }
fn e25_45(d: &[u8]) -> u64 { exp_swap_core(d, 45.0, 0.25) }
fn e25_50(d: &[u8]) -> u64 { exp_swap_core(d, 50.0, 0.25) }
fn e25_52(d: &[u8]) -> u64 { exp_swap_core(d, 52.0, 0.25) }
fn e25_55(d: &[u8]) -> u64 { exp_swap_core(d, 55.0, 0.25) }

fn e27_45(d: &[u8]) -> u64 { exp_swap_core(d, 45.0, 0.27) }
fn e27_50(d: &[u8]) -> u64 { exp_swap_core(d, 50.0, 0.27) }
fn e27_52(d: &[u8]) -> u64 { exp_swap_core(d, 52.0, 0.27) }
fn e27_55(d: &[u8]) -> u64 { exp_swap_core(d, 55.0, 0.27) }
fn e27_57(d: &[u8]) -> u64 { exp_swap_core(d, 57.0, 0.27) }

fn e30_45(d: &[u8]) -> u64 { exp_swap_core(d, 45.0, 0.30) }
fn e30_50(d: &[u8]) -> u64 { exp_swap_core(d, 50.0, 0.30) }
fn e30_52(d: &[u8]) -> u64 { exp_swap_core(d, 52.0, 0.30) }
fn e30_55(d: &[u8]) -> u64 { exp_swap_core(d, 55.0, 0.30) }
fn e30_57(d: &[u8]) -> u64 { exp_swap_core(d, 57.0, 0.30) }
fn e30_60(d: &[u8]) -> u64 { exp_swap_core(d, 60.0, 0.30) }

fn e32_50(d: &[u8]) -> u64 { exp_swap_core(d, 50.0, 0.32) }
fn e32_55(d: &[u8]) -> u64 { exp_swap_core(d, 55.0, 0.32) }
fn e32_57(d: &[u8]) -> u64 { exp_swap_core(d, 57.0, 0.32) }
fn e32_60(d: &[u8]) -> u64 { exp_swap_core(d, 60.0, 0.32) }

fn e35_50(d: &[u8]) -> u64 { exp_swap_core(d, 50.0, 0.35) }
fn e35_55(d: &[u8]) -> u64 { exp_swap_core(d, 55.0, 0.35) }
fn e35_60(d: &[u8]) -> u64 { exp_swap_core(d, 60.0, 0.35) }

fn run_test(name: &str, swap_fn: fn(&[u8]) -> u64, n_sims: u32) -> f64 {
    let result = runner::run_default_batch_native(
        swap_fn, None, normalizer_swap, Some(normalizer_after_swap),
        n_sims, 10_000, None,
    ).unwrap();
    println!("{:>25}: avg={:>8.2}", name, result.avg_edge());
    result.avg_edge()
}

fn main() {
    let n = 1000u32;
    
    println!("=== Fine Sweep: Exp(α, fee) ===");
    let mut best = ("", 0.0f64);
    
    let tests: Vec<(&str, fn(&[u8]) -> u64)> = vec![
        ("e22 40bp", e22_40), ("e22 45bp", e22_45), ("e22 50bp", e22_50), ("e22 55bp", e22_55),
        ("e25 40bp", e25_40), ("e25 45bp", e25_45), ("e25 50bp", e25_50), ("e25 52bp", e25_52), ("e25 55bp", e25_55),
        ("e27 45bp", e27_45), ("e27 50bp", e27_50), ("e27 52bp", e27_52), ("e27 55bp", e27_55), ("e27 57bp", e27_57),
        ("e30 45bp", e30_45), ("e30 50bp", e30_50), ("e30 52bp", e30_52), ("e30 55bp", e30_55), ("e30 57bp", e30_57), ("e30 60bp", e30_60),
        ("e32 50bp", e32_50), ("e32 55bp", e32_55), ("e32 57bp", e32_57), ("e32 60bp", e32_60),
        ("e35 50bp", e35_50), ("e35 55bp", e35_55), ("e35 60bp", e35_60),
    ];
    
    for (name, f) in &tests {
        let score = run_test(name, *f, n);
        if score > best.1 { best = (name, score); }
    }
    
    println!("\nBest: {} = {:.2}", best.0, best.1);
}
