use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_sim::runner;

const NANO: f64 = 1_000_000_000.0;

fn exp_swap(data: &[u8], fee_bps: f64, alpha: f64) -> u64 {
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

// Try: fee based on trade size AND reserve deviation (combined signals)
fn smart_swap(data: &[u8], base_fee: f64, alpha: f64, size_scale: f64, dev_scale: f64) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return 0; }
    
    // Size-based fee adjustment: larger trades get higher fee
    let (ri, ro) = match side { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    let rel_size = input / ri;
    let size_adj = 1.0 + size_scale * rel_size;
    
    // Deviation-based fee adjustment
    let spot = ry / rx;
    let dev = ((spot / 100.0).ln()).abs();
    let dev_adj = 1.0 + dev_scale * dev;
    
    let fee_bps = (base_fee * size_adj * dev_adj).max(15.0).min(400.0);
    let gamma = (10000.0 - fee_bps) / 10000.0;
    
    let u = gamma * input / (ri * alpha);
    let output = alpha * ro * (1.0 - (-u).exp());
    if output <= 0.0 || !output.is_finite() { return 0; }
    let scaled = (output.min(ro * 0.999) * NANO).floor();
    if scaled <= 0.0 || scaled >= u64::MAX as f64 { 0 } else { scaled as u64 }
}

// Extreme low fees
fn e030_15(d: &[u8]) -> u64 { exp_swap(d, 15.0, 0.30) }
fn e030_20(d: &[u8]) -> u64 { exp_swap(d, 20.0, 0.30) }
fn e030_25(d: &[u8]) -> u64 { exp_swap(d, 25.0, 0.30) }
fn e030_30(d: &[u8]) -> u64 { exp_swap(d, 30.0, 0.30) }
fn e030_35(d: &[u8]) -> u64 { exp_swap(d, 35.0, 0.30) }
fn e030_55(d: &[u8]) -> u64 { exp_swap(d, 55.0, 0.30) }

// Very small alpha (extreme concavity)
fn e010_40(d: &[u8]) -> u64 { exp_swap(d, 40.0, 0.10) }
fn e010_50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.10) }
fn e005_40(d: &[u8]) -> u64 { exp_swap(d, 40.0, 0.05) }
fn e005_50(d: &[u8]) -> u64 { exp_swap(d, 50.0, 0.05) }

// Smart swap configurations
fn smart_50_03_5_3(d: &[u8]) -> u64 { smart_swap(d, 50.0, 0.3, 5.0, 3.0) }
fn smart_45_03_10_3(d: &[u8]) -> u64 { smart_swap(d, 45.0, 0.3, 10.0, 3.0) }
fn smart_40_03_10_5(d: &[u8]) -> u64 { smart_swap(d, 40.0, 0.3, 10.0, 5.0) }
fn smart_50_03_10_3(d: &[u8]) -> u64 { smart_swap(d, 50.0, 0.3, 10.0, 3.0) }
fn smart_55_03_5_5(d: &[u8]) -> u64 { smart_swap(d, 55.0, 0.3, 5.0, 5.0) }
fn smart_40_03_20_5(d: &[u8]) -> u64 { smart_swap(d, 40.0, 0.3, 20.0, 5.0) }
fn smart_35_03_20_5(d: &[u8]) -> u64 { smart_swap(d, 35.0, 0.3, 20.0, 5.0) }
fn smart_35_03_30_5(d: &[u8]) -> u64 { smart_swap(d, 35.0, 0.3, 30.0, 5.0) }
fn smart_30_03_30_5(d: &[u8]) -> u64 { smart_swap(d, 30.0, 0.3, 30.0, 5.0) }
fn smart_30_03_50_5(d: &[u8]) -> u64 { smart_swap(d, 30.0, 0.3, 50.0, 5.0) }
fn smart_30_025_50_5(d: &[u8]) -> u64 { smart_swap(d, 30.0, 0.25, 50.0, 5.0) }
fn smart_30_025_50_8(d: &[u8]) -> u64 { smart_swap(d, 30.0, 0.25, 50.0, 8.0) }
fn smart_25_025_50_5(d: &[u8]) -> u64 { smart_swap(d, 25.0, 0.25, 50.0, 5.0) }
fn smart_30_03_50_8(d: &[u8]) -> u64 { smart_swap(d, 30.0, 0.3, 50.0, 8.0) }

fn run_test(name: &str, swap_fn: fn(&[u8]) -> u64, n_sims: u32) {
    let result = runner::run_default_batch_native(
        swap_fn, None, normalizer_swap, Some(normalizer_after_swap),
        n_sims, 10_000, None,
    ).unwrap();
    println!("{:>30}: avg={:>8.2}", name, result.avg_edge());
}

fn main() {
    let n = 1000u32;
    
    println!("=== Extreme Low Fees ===");
    run_test("exp030 15bp", e030_15, n);
    run_test("exp030 20bp", e030_20, n);
    run_test("exp030 25bp", e030_25, n);
    run_test("exp030 30bp", e030_30, n);
    run_test("exp030 35bp", e030_35, n);
    run_test("exp030 55bp", e030_55, n);

    println!("\n=== Extreme Small Alpha ===");
    run_test("exp010 40bp", e010_40, n);
    run_test("exp010 50bp", e010_50, n);
    run_test("exp005 40bp", e005_40, n);
    run_test("exp005 50bp", e005_50, n);
    
    println!("\n=== Smart: size + deviation adaptive ===");
    run_test("s50 a03 sz5 d3", smart_50_03_5_3, n);
    run_test("s45 a03 sz10 d3", smart_45_03_10_3, n);
    run_test("s40 a03 sz10 d5", smart_40_03_10_5, n);
    run_test("s50 a03 sz10 d3", smart_50_03_10_3, n);
    run_test("s55 a03 sz5 d5", smart_55_03_5_5, n);
    run_test("s40 a03 sz20 d5", smart_40_03_20_5, n);
    run_test("s35 a03 sz20 d5", smart_35_03_20_5, n);
    run_test("s35 a03 sz30 d5", smart_35_03_30_5, n);
    run_test("s30 a03 sz30 d5", smart_30_03_30_5, n);
    run_test("s30 a03 sz50 d5", smart_30_03_50_5, n);
    run_test("s30 a025 sz50 d5", smart_30_025_50_5, n);
    run_test("s30 a025 sz50 d8", smart_30_025_50_8, n);
    run_test("s25 a025 sz50 d5", smart_25_025_50_5, n);
    run_test("s30 a03 sz50 d8", smart_30_03_50_8, n);
}
