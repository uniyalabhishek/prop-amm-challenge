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

// Smooth piecewise: blend between CP and exp based on input size
// For small inputs: output ≈ CP (good depth for retail)
// For large inputs: output ≈ exp (more concave, arb protection)
// Blend weight: w(Y) = 1 / (1 + (Y / Y_scale)^2)
// output = w * CP_output + (1-w) * exp_output
fn blended_swap(data: &[u8], fee_bps: f64, alpha_exp: f64, y_scale: f64) -> u64 {
    let (side, input, rx, ry) = match parse_data(data) {
        Some(v) => v,
        None => return 0,
    };
    let gamma = (10000.0 - fee_bps) / 10000.0;
    
    let (res_in, res_out) = match side {
        0 => (ry, rx),  // Buy X: input Y, output X
        1 => (rx, ry),  // Sell X: input X, output Y
        _ => return 0,
    };
    
    // CP output
    let net = input * gamma;
    let cp_out = res_out * net / (res_in + net);
    
    // Exp output
    let u = gamma * input / (res_in * alpha_exp);
    let exp_out = alpha_exp * res_out * (1.0 - (-u).exp());
    
    // Blend weight based on trade size relative to reserves
    let y_rel = input / y_scale;
    let w = 1.0 / (1.0 + y_rel * y_rel);
    
    let output = w * cp_out + (1.0 - w) * exp_out;
    
    if output <= 0.0 || !output.is_finite() { return 0; }
    to_nano_u64(output.min(res_out * 0.999))
}

// Trade-size dependent fee: lower fee for small trades, higher for large
fn size_fee_swap(data: &[u8], fee_low: f64, fee_high: f64, alpha: f64, y_scale: f64) -> u64 {
    let (side, input, rx, ry) = match parse_data(data) {
        Some(v) => v,
        None => return 0,
    };
    
    let y_rel = input / y_scale;
    let w = 1.0 / (1.0 + y_rel);
    let fee_bps = w * fee_low + (1.0 - w) * fee_high;
    let gamma = (10000.0 - fee_bps) / 10000.0;
    
    let (res_in, res_out) = match side {
        0 => (ry, rx),
        1 => (rx, ry),
        _ => return 0,
    };
    
    let u = gamma * input / (res_in * alpha);
    let output = alpha * res_out * (1.0 - (-u).exp());
    
    if output <= 0.0 || !output.is_finite() { return 0; }
    to_nano_u64(output.min(res_out * 0.999))
}

// Also try: compute fee from reserve state (k-ratio)
fn k_adaptive_exp_swap(data: &[u8], base_fee: f64, alpha: f64, sensitivity: f64) -> u64 {
    let (side, input, rx, ry) = match parse_data(data) {
        Some(v) => v,
        None => return 0,
    };
    
    // k-ratio: current product vs initial (100*10000 = 1M)
    let k_ratio = (rx * ry) / 1_000_000.0;
    
    // Higher k = more fee collected = more trading = higher vol
    // Actually: k always increases. Higher k could mean we're doing well.
    // Use price deviation instead as vol proxy
    let spot = ry / rx;
    let dev = ((spot / 100.0).ln()).abs();
    
    // fee = base * (1 + sensitivity * dev)
    let fee_bps = base_fee * (1.0 + sensitivity * dev);
    let fee_bps = fee_bps.max(25.0).min(300.0);
    let gamma = (10000.0 - fee_bps) / 10000.0;
    
    let (res_in, res_out) = match side {
        0 => (ry, rx),
        1 => (rx, ry),
        _ => return 0,
    };
    
    let u = gamma * input / (res_in * alpha);
    let output = alpha * res_out * (1.0 - (-u).exp());
    
    if output <= 0.0 || !output.is_finite() { return 0; }
    to_nano_u64(output.min(res_out * 0.999))
}

// Define concrete functions
fn blend_55_03_50(d: &[u8]) -> u64 { blended_swap(d, 55.0, 0.3, 50.0) }
fn blend_55_03_100(d: &[u8]) -> u64 { blended_swap(d, 55.0, 0.3, 100.0) }
fn blend_55_03_200(d: &[u8]) -> u64 { blended_swap(d, 55.0, 0.3, 200.0) }
fn blend_55_02_50(d: &[u8]) -> u64 { blended_swap(d, 55.0, 0.2, 50.0) }
fn blend_55_02_100(d: &[u8]) -> u64 { blended_swap(d, 55.0, 0.2, 100.0) }
fn blend_60_03_100(d: &[u8]) -> u64 { blended_swap(d, 60.0, 0.3, 100.0) }
fn blend_50_03_100(d: &[u8]) -> u64 { blended_swap(d, 50.0, 0.3, 100.0) }
fn blend_55_025_100(d: &[u8]) -> u64 { blended_swap(d, 55.0, 0.25, 100.0) }

fn sfee_30_80_03_30(d: &[u8]) -> u64 { size_fee_swap(d, 30.0, 80.0, 0.3, 30.0) }
fn sfee_30_80_03_50(d: &[u8]) -> u64 { size_fee_swap(d, 30.0, 80.0, 0.3, 50.0) }
fn sfee_40_80_03_50(d: &[u8]) -> u64 { size_fee_swap(d, 40.0, 80.0, 0.3, 50.0) }
fn sfee_40_100_03_50(d: &[u8]) -> u64 { size_fee_swap(d, 40.0, 100.0, 0.3, 50.0) }
fn sfee_30_100_03_30(d: &[u8]) -> u64 { size_fee_swap(d, 30.0, 100.0, 0.3, 30.0) }
fn sfee_30_120_03_30(d: &[u8]) -> u64 { size_fee_swap(d, 30.0, 120.0, 0.3, 30.0) }
fn sfee_40_120_025_50(d: &[u8]) -> u64 { size_fee_swap(d, 40.0, 120.0, 0.25, 50.0) }
fn sfee_35_100_03_40(d: &[u8]) -> u64 { size_fee_swap(d, 35.0, 100.0, 0.3, 40.0) }
fn sfee_45_100_03_40(d: &[u8]) -> u64 { size_fee_swap(d, 45.0, 100.0, 0.3, 40.0) }
fn sfee_35_90_03_40(d: &[u8]) -> u64 { size_fee_swap(d, 35.0, 90.0, 0.3, 40.0) }
fn sfee_35_80_03_40(d: &[u8]) -> u64 { size_fee_swap(d, 35.0, 80.0, 0.3, 40.0) }

fn kadapt_55_03_3(d: &[u8]) -> u64 { k_adaptive_exp_swap(d, 55.0, 0.3, 3.0) }
fn kadapt_55_03_5(d: &[u8]) -> u64 { k_adaptive_exp_swap(d, 55.0, 0.3, 5.0) }
fn kadapt_55_03_8(d: &[u8]) -> u64 { k_adaptive_exp_swap(d, 55.0, 0.3, 8.0) }
fn kadapt_50_03_5(d: &[u8]) -> u64 { k_adaptive_exp_swap(d, 50.0, 0.3, 5.0) }
fn kadapt_40_03_5(d: &[u8]) -> u64 { k_adaptive_exp_swap(d, 40.0, 0.3, 5.0) }

fn run_test(name: &str, swap_fn: fn(&[u8]) -> u64, n_sims: u32) -> f64 {
    let result = runner::run_default_batch_native(
        swap_fn, None, normalizer_swap, Some(normalizer_after_swap),
        n_sims, 10_000, None,
    ).unwrap();
    println!("{:>30}: avg={:>8.2}", name, result.avg_edge());
    result.avg_edge()
}

fn main() {
    let n = 1000u32;
    
    println!("=== Blended CP+Exp ===");
    run_test("blend 55 a03 s50", blend_55_03_50, n);
    run_test("blend 55 a03 s100", blend_55_03_100, n);
    run_test("blend 55 a03 s200", blend_55_03_200, n);
    run_test("blend 55 a02 s50", blend_55_02_50, n);
    run_test("blend 55 a02 s100", blend_55_02_100, n);
    run_test("blend 60 a03 s100", blend_60_03_100, n);
    run_test("blend 50 a03 s100", blend_50_03_100, n);
    run_test("blend 55 a025 s100", blend_55_025_100, n);
    
    println!("\n=== Size-dependent Fee ===");
    run_test("sfee 30-80 a03 s30", sfee_30_80_03_30, n);
    run_test("sfee 30-80 a03 s50", sfee_30_80_03_50, n);
    run_test("sfee 40-80 a03 s50", sfee_40_80_03_50, n);
    run_test("sfee 40-100 a03 s50", sfee_40_100_03_50, n);
    run_test("sfee 30-100 a03 s30", sfee_30_100_03_30, n);
    run_test("sfee 30-120 a03 s30", sfee_30_120_03_30, n);
    run_test("sfee 40-120 a025 s50", sfee_40_120_025_50, n);
    run_test("sfee 35-100 a03 s40", sfee_35_100_03_40, n);
    run_test("sfee 45-100 a03 s40", sfee_45_100_03_40, n);
    run_test("sfee 35-90 a03 s40", sfee_35_90_03_40, n);
    run_test("sfee 35-80 a03 s40", sfee_35_80_03_40, n);
    
    println!("\n=== K-adaptive Exp ===");
    run_test("kadapt 55 a03 s3", kadapt_55_03_3, n);
    run_test("kadapt 55 a03 s5", kadapt_55_03_5, n);
    run_test("kadapt 55 a03 s8", kadapt_55_03_8, n);
    run_test("kadapt 50 a03 s5", kadapt_50_03_5, n);
    run_test("kadapt 40 a03 s5", kadapt_40_03_5, n);
}
