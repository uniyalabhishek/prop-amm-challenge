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

// Exponential curve
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

// Exp with various alpha
fn e_a020_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.20) }
fn e_a020_f55(d: &[u8]) -> u64 { exp_swap(d, 55.0, 0.20) }
fn e_a020_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.20) }
fn e_a020_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.20) }
fn e_a020_f70(d: &[u8]) -> u64 { exp_swap(d, 70.0, 0.20) }

fn e_a025_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.25) }
fn e_a025_f55(d: &[u8]) -> u64 { exp_swap(d, 55.0, 0.25) }
fn e_a025_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.25) }
fn e_a025_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.25) }
fn e_a025_f70(d: &[u8]) -> u64 { exp_swap(d, 70.0, 0.25) }

fn e_a030_f45(d: &[u8]) -> u64 { exp_swap(d, 45.0, 0.30) }
fn e_a030_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.30) }
fn e_a030_f55(d: &[u8]) -> u64 { exp_swap(d, 55.0, 0.30) }
fn e_a030_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.30) }
fn e_a030_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.30) }

fn e_a035_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.35) }
fn e_a035_f55(d: &[u8]) -> u64 { exp_swap(d, 55.0, 0.35) }
fn e_a035_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.35) }
fn e_a035_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.35) }

fn e_a040_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.40) }
fn e_a040_f55(d: &[u8]) -> u64 { exp_swap(d, 55.0, 0.40) }
fn e_a040_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.40) }
fn e_a040_f65(d: &[u8]) -> u64 { exp_swap(d, 65.0, 0.40) }

fn e_a015_f50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.15) }
fn e_a015_f55(d: &[u8]) -> u64 { exp_swap(d, 55.0, 0.15) }
fn e_a015_f60(d: &[u8]) -> u64 { exp_swap(d, 60.0, 0.15) }

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
    
    println!("=== Exp α=0.15 ===");
    run_test("Exp 0.15 50bp", e_a015_f50, n);
    run_test("Exp 0.15 55bp", e_a015_f55, n);
    run_test("Exp 0.15 60bp", e_a015_f60, n);
    
    println!("\n=== Exp α=0.20 ===");
    run_test("Exp 0.20 50bp", e_a020_f50, n);
    run_test("Exp 0.20 55bp", e_a020_f55, n);
    run_test("Exp 0.20 60bp", e_a020_f60, n);
    run_test("Exp 0.20 65bp", e_a020_f65, n);
    run_test("Exp 0.20 70bp", e_a020_f70, n);
    
    println!("\n=== Exp α=0.25 ===");
    run_test("Exp 0.25 50bp", e_a025_f50, n);
    run_test("Exp 0.25 55bp", e_a025_f55, n);
    run_test("Exp 0.25 60bp", e_a025_f60, n);
    run_test("Exp 0.25 65bp", e_a025_f65, n);
    run_test("Exp 0.25 70bp", e_a025_f70, n);
    
    println!("\n=== Exp α=0.30 ===");
    run_test("Exp 0.30 45bp", e_a030_f45, n);
    run_test("Exp 0.30 50bp", e_a030_f50, n);
    run_test("Exp 0.30 55bp", e_a030_f55, n);
    run_test("Exp 0.30 60bp", e_a030_f60, n);
    run_test("Exp 0.30 65bp", e_a030_f65, n);
    
    println!("\n=== Exp α=0.35 ===");
    run_test("Exp 0.35 50bp", e_a035_f50, n);
    run_test("Exp 0.35 55bp", e_a035_f55, n);
    run_test("Exp 0.35 60bp", e_a035_f60, n);
    run_test("Exp 0.35 65bp", e_a035_f65, n);
    
    println!("\n=== Exp α=0.40 ===");
    run_test("Exp 0.40 50bp", e_a040_f50, n);
    run_test("Exp 0.40 55bp", e_a040_f55, n);
    run_test("Exp 0.40 60bp", e_a040_f60, n);
    run_test("Exp 0.40 65bp", e_a040_f65, n);
}
