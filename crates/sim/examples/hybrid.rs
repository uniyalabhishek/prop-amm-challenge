use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_sim::runner;

const NANO: f64 = 1_000_000_000.0;

fn parse(data: &[u8]) -> Option<(u8, f64, f64, f64)> {
    if data.len() < 25 { return None; }
    let s = data[0];
    let i = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let x = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let y = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if i <= 0.0 || x <= 0.0 || y <= 0.0 { return None; }
    Some((s, i, x, y))
}

fn to_nano(v: f64) -> u64 {
    if v <= 0.0 || !v.is_finite() { return 0; }
    let s = (v * NANO).floor();
    if s >= u64::MAX as f64 { u64::MAX } else { s as u64 }
}

// Best static: exp(0.30, 55bp) → 449.75
fn exp_static(d: &[u8]) -> u64 {
    let (s, i, rx, ry) = match parse(d) { Some(v) => v, None => return 0 };
    let g = 0.9945; let a = 0.30;
    let (ri, ro) = match s { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    let u = g * i / (ri * a);
    let o = a * ro * (1.0 - (-u).exp());
    to_nano(o.min(ro * 0.999))
}

// What if we use a DIFFERENT formula that maintains the same marginal price
// but has a DIFFERENT concavity profile?
// Formula: output = reserve_out * input * gamma / (reserve_in + input * gamma + beta * input^2)
// This adds a quadratic penalty that increases with trade size
// Marginal at 0: gamma * reserve_out / reserve_in (same as CP)
// More concave than CP for large inputs (quadratic term dominates)
fn quad_penalty(d: &[u8], fee_bps: f64, beta_scale: f64) -> u64 {
    let (s, i, rx, ry) = match parse(d) { Some(v) => v, None => return 0 };
    let g = (10000.0 - fee_bps) / 10000.0;
    let (ri, ro) = match s { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    let beta = beta_scale / ri; // normalize by reserve size
    let denom = ri + g * i + beta * i * i;
    if denom <= 0.0 { return 0; }
    let o = ro * g * i / denom;
    if o <= 0.0 || !o.is_finite() { return 0; }
    to_nano(o.min(ro * 0.999))
}

// Let me also test: output = reserve_out * gamma * input / (reserve_in + gamma * input)^(1+eps)
// For eps=0: standard CP
// For eps>0: more concave
// But need to handle the math carefully for concavity
fn cp_power(d: &[u8], fee_bps: f64, eps: f64) -> u64 {
    let (s, i, rx, ry) = match parse(d) { Some(v) => v, None => return 0 };
    let g = (10000.0 - fee_bps) / 10000.0;
    let (ri, ro) = match s { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    // output = ro * (1 - (ri / (ri + g*i))^(1+eps))
    // Marginal at 0: (1+eps) * g * ro / ri
    // To maintain same marginal as CP: divide output by (1+eps)
    let ratio = ri / (ri + g * i);
    let o = ro * (1.0 - ratio.powf(1.0 + eps)) / (1.0 + eps);
    if o <= 0.0 || !o.is_finite() { return 0; }
    to_nano(o.min(ro * 0.999))
}

// Quadratic penalty variants
fn qp_55_001(d: &[u8]) -> u64 { quad_penalty(d, 55.0, 0.001) }
fn qp_55_005(d: &[u8]) -> u64 { quad_penalty(d, 55.0, 0.005) }
fn qp_55_01(d: &[u8]) -> u64 { quad_penalty(d, 55.0, 0.01) }
fn qp_55_02(d: &[u8]) -> u64 { quad_penalty(d, 55.0, 0.02) }
fn qp_55_05(d: &[u8]) -> u64 { quad_penalty(d, 55.0, 0.05) }
fn qp_55_1(d: &[u8]) -> u64 { quad_penalty(d, 55.0, 0.1) }
fn qp_55_2(d: &[u8]) -> u64 { quad_penalty(d, 55.0, 0.2) }
fn qp_55_5(d: &[u8]) -> u64 { quad_penalty(d, 55.0, 0.5) }
fn qp_50_01(d: &[u8]) -> u64 { quad_penalty(d, 50.0, 0.01) }
fn qp_50_02(d: &[u8]) -> u64 { quad_penalty(d, 50.0, 0.02) }
fn qp_50_05(d: &[u8]) -> u64 { quad_penalty(d, 50.0, 0.05) }
fn qp_50_1(d: &[u8]) -> u64 { quad_penalty(d, 50.0, 0.1) }
fn qp_60_01(d: &[u8]) -> u64 { quad_penalty(d, 60.0, 0.01) }
fn qp_60_02(d: &[u8]) -> u64 { quad_penalty(d, 60.0, 0.02) }
fn qp_60_05(d: &[u8]) -> u64 { quad_penalty(d, 60.0, 0.05) }

// CP power variants
fn cpp_55_03(d: &[u8]) -> u64 { cp_power(d, 55.0, 0.3) }
fn cpp_55_05(d: &[u8]) -> u64 { cp_power(d, 55.0, 0.5) }
fn cpp_55_1(d: &[u8]) -> u64 { cp_power(d, 55.0, 1.0) }
fn cpp_55_2(d: &[u8]) -> u64 { cp_power(d, 55.0, 2.0) }
fn cpp_55_3(d: &[u8]) -> u64 { cp_power(d, 55.0, 3.0) }
fn cpp_55_5(d: &[u8]) -> u64 { cp_power(d, 55.0, 5.0) }
fn cpp_50_1(d: &[u8]) -> u64 { cp_power(d, 50.0, 1.0) }
fn cpp_50_2(d: &[u8]) -> u64 { cp_power(d, 50.0, 2.0) }
fn cpp_50_3(d: &[u8]) -> u64 { cp_power(d, 50.0, 3.0) }
fn cpp_60_1(d: &[u8]) -> u64 { cp_power(d, 60.0, 1.0) }
fn cpp_60_2(d: &[u8]) -> u64 { cp_power(d, 60.0, 2.0) }
fn cpp_60_3(d: &[u8]) -> u64 { cp_power(d, 60.0, 3.0) }

fn run(name: &str, f: fn(&[u8]) -> u64, n: u32) {
    let r = runner::run_default_batch_native(
        f, None, normalizer_swap, Some(normalizer_after_swap), n, 10_000, None).unwrap();
    println!("{:>25}: avg={:>8.2}", name, r.avg_edge());
}

fn main() {
    let n = 1000u32;
    
    println!("=== Baseline ===");
    run("exp static 0.30 55bp", exp_static, n);
    
    println!("\n=== Quadratic Penalty ===");
    run("qp 55 β=0.001", qp_55_001, n);
    run("qp 55 β=0.005", qp_55_005, n);
    run("qp 55 β=0.01", qp_55_01, n);
    run("qp 55 β=0.02", qp_55_02, n);
    run("qp 55 β=0.05", qp_55_05, n);
    run("qp 55 β=0.1", qp_55_1, n);
    run("qp 55 β=0.2", qp_55_2, n);
    run("qp 55 β=0.5", qp_55_5, n);
    run("qp 50 β=0.01", qp_50_01, n);
    run("qp 50 β=0.02", qp_50_02, n);
    run("qp 50 β=0.05", qp_50_05, n);
    run("qp 50 β=0.1", qp_50_1, n);
    run("qp 60 β=0.01", qp_60_01, n);
    run("qp 60 β=0.02", qp_60_02, n);
    run("qp 60 β=0.05", qp_60_05, n);
    
    println!("\n=== CP Power ===");
    run("cpp 55 ε=0.3", cpp_55_03, n);
    run("cpp 55 ε=0.5", cpp_55_05, n);
    run("cpp 55 ε=1.0", cpp_55_1, n);
    run("cpp 55 ε=2.0", cpp_55_2, n);
    run("cpp 55 ε=3.0", cpp_55_3, n);
    run("cpp 55 ε=5.0", cpp_55_5, n);
    run("cpp 50 ε=1.0", cpp_50_1, n);
    run("cpp 50 ε=2.0", cpp_50_2, n);
    run("cpp 50 ε=3.0", cpp_50_3, n);
    run("cpp 60 ε=1.0", cpp_60_1, n);
    run("cpp 60 ε=2.0", cpp_60_2, n);
    run("cpp 60 ε=3.0", cpp_60_3, n);
}
