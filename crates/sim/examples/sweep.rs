use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_shared::config::{HyperparameterVariance, SimulationConfig};
use prop_amm_sim::runner;
use prop_amm_sim::engine;
use std::time::Instant;

const NANO: f64 = 1_000_000_000.0;

fn cp_swap_core(data: &[u8], fee_bps: u128) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as u128;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as u128;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as u128;
    if rx == 0 || ry == 0 { return 0; }
    let gamma_num = 10_000u128 - fee_bps;
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

// Virtual reserves: compute as if reserves are multiplied by virt_mult
fn virtual_cp_swap_core(data: &[u8], fee_bps: u128, virt_mult_num: u128, virt_mult_den: u128) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as u128;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as u128;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as u128;
    if rx == 0 || ry == 0 { return 0; }
    let gamma_num = 10_000u128 - fee_bps;
    
    let eff_rx = rx * virt_mult_num / virt_mult_den;
    let eff_ry = ry * virt_mult_num / virt_mult_den;
    if eff_rx == 0 || eff_ry == 0 { return 0; }
    let k = eff_rx * eff_ry;
    
    match side {
        0 => {
            let net = input * gamma_num / 10_000;
            let new_ry = eff_ry + net;
            let out = eff_rx.saturating_sub((k + new_ry - 1) / new_ry);
            out.min(rx.saturating_sub(1)) as u64
        }
        1 => {
            let net = input * gamma_num / 10_000;
            let new_rx = eff_rx + net;
            let out = eff_ry.saturating_sub((k + new_rx - 1) / new_rx);
            out.min(ry.saturating_sub(1)) as u64
        }
        _ => 0,
    }
}

// Define concrete swap functions
fn cp_30(data: &[u8]) -> u64 { cp_swap_core(data, 30) }
fn cp_40(data: &[u8]) -> u64 { cp_swap_core(data, 40) }
fn cp_50(data: &[u8]) -> u64 { cp_swap_core(data, 50) }
fn cp_55(data: &[u8]) -> u64 { cp_swap_core(data, 55) }
fn cp_60(data: &[u8]) -> u64 { cp_swap_core(data, 60) }
fn cp_65(data: &[u8]) -> u64 { cp_swap_core(data, 65) }
fn cp_70(data: &[u8]) -> u64 { cp_swap_core(data, 70) }
fn cp_75(data: &[u8]) -> u64 { cp_swap_core(data, 75) }
fn cp_80(data: &[u8]) -> u64 { cp_swap_core(data, 80) }
fn cp_90(data: &[u8]) -> u64 { cp_swap_core(data, 90) }
fn cp_100(data: &[u8]) -> u64 { cp_swap_core(data, 100) }
fn cp_120(data: &[u8]) -> u64 { cp_swap_core(data, 120) }

// Virtual reserves 2x with different fees
fn vcp_2x_40(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 40, 20, 10) }
fn vcp_2x_50(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 50, 20, 10) }
fn vcp_2x_60(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 60, 20, 10) }
fn vcp_2x_65(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 65, 20, 10) }
fn vcp_2x_70(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 70, 20, 10) }
fn vcp_2x_80(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 80, 20, 10) }
fn vcp_2x_90(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 90, 20, 10) }
fn vcp_2x_100(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 100, 20, 10) }
fn vcp_2x_120(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 120, 20, 10) }

// Virtual reserves 3x
fn vcp_3x_50(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 50, 30, 10) }
fn vcp_3x_65(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 65, 30, 10) }
fn vcp_3x_80(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 80, 30, 10) }
fn vcp_3x_100(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 100, 30, 10) }
fn vcp_3x_120(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 120, 30, 10) }
fn vcp_3x_150(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 150, 30, 10) }

// Virtual reserves 5x
fn vcp_5x_80(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 80, 50, 10) }
fn vcp_5x_100(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 100, 50, 10) }
fn vcp_5x_120(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 120, 50, 10) }
fn vcp_5x_150(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 150, 50, 10) }
fn vcp_5x_200(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 200, 50, 10) }

// Virtual reserves 1.5x
fn vcp_15x_50(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 50, 15, 10) }
fn vcp_15x_60(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 60, 15, 10) }
fn vcp_15x_65(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 65, 15, 10) }
fn vcp_15x_70(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 70, 15, 10) }
fn vcp_15x_80(data: &[u8]) -> u64 { virtual_cp_swap_core(data, 80, 15, 10) }

// Dynamic fee based on price deviation from initial
fn dynamic_price_dev(data: &[u8]) -> u64 {
    if data.len() < 25 { return 0; }
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64;
    if rx == 0.0 || ry == 0.0 { return 0; }
    
    // Spot price = ry/rx. Initial = NANO*10000 / (NANO*100) = 100.0
    let spot = ry / rx;
    let dev = ((spot / 100.0).ln()).abs();
    
    let fee_bps = if dev < 0.005 { 50u128 }
    else if dev < 0.02 { 60 }
    else if dev < 0.05 { 70 }
    else if dev < 0.10 { 85 }
    else if dev < 0.20 { 100 }
    else { 140 };
    
    cp_swap_core(data, fee_bps)
}

// Dynamic fee with smoother formula
fn dynamic_smooth(data: &[u8]) -> u64 {
    if data.len() < 25 { return 0; }
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64;
    if rx == 0.0 || ry == 0.0 { return 0; }
    
    let spot = ry / rx;
    let dev = ((spot / 100.0).ln()).abs();
    
    // fee_bps = base + scale * dev
    let fee_f64 = 50.0 + 500.0 * dev;
    let fee_bps = (fee_f64 as u128).max(30).min(300);
    
    cp_swap_core(data, fee_bps)
}

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
    println!("{:>30}: avg_edge={:>8.2}  total={:>10.2}  time={:.2}s", 
        name, result.avg_edge(), result.total_edge, elapsed.as_secs_f64());
}

fn main() {
    let n = 1000u32;
    
    println!("=== Fee Sweep ===");
    run_test("CP 30bp", cp_30, n);
    run_test("CP 40bp", cp_40, n);
    run_test("CP 50bp", cp_50, n);
    run_test("CP 55bp", cp_55, n);
    run_test("CP 60bp", cp_60, n);
    run_test("CP 65bp", cp_65, n);
    run_test("CP 70bp", cp_70, n);
    run_test("CP 75bp", cp_75, n);
    run_test("CP 80bp", cp_80, n);
    run_test("CP 90bp", cp_90, n);
    run_test("CP 100bp", cp_100, n);
    run_test("CP 120bp", cp_120, n);
    
    println!("\n=== Virtual 1.5x + Fee ===");
    run_test("V1.5x 50bp", vcp_15x_50, n);
    run_test("V1.5x 60bp", vcp_15x_60, n);
    run_test("V1.5x 65bp", vcp_15x_65, n);
    run_test("V1.5x 70bp", vcp_15x_70, n);
    run_test("V1.5x 80bp", vcp_15x_80, n);
    
    println!("\n=== Virtual 2x + Fee ===");
    run_test("V2x 40bp", vcp_2x_40, n);
    run_test("V2x 50bp", vcp_2x_50, n);
    run_test("V2x 60bp", vcp_2x_60, n);
    run_test("V2x 65bp", vcp_2x_65, n);
    run_test("V2x 70bp", vcp_2x_70, n);
    run_test("V2x 80bp", vcp_2x_80, n);
    run_test("V2x 90bp", vcp_2x_90, n);
    run_test("V2x 100bp", vcp_2x_100, n);
    run_test("V2x 120bp", vcp_2x_120, n);
    
    println!("\n=== Virtual 3x + Fee ===");
    run_test("V3x 50bp", vcp_3x_50, n);
    run_test("V3x 65bp", vcp_3x_65, n);
    run_test("V3x 80bp", vcp_3x_80, n);
    run_test("V3x 100bp", vcp_3x_100, n);
    run_test("V3x 120bp", vcp_3x_120, n);
    run_test("V3x 150bp", vcp_3x_150, n);
    
    println!("\n=== Virtual 5x + Fee ===");
    run_test("V5x 80bp", vcp_5x_80, n);
    run_test("V5x 100bp", vcp_5x_100, n);
    run_test("V5x 120bp", vcp_5x_120, n);
    run_test("V5x 150bp", vcp_5x_150, n);
    run_test("V5x 200bp", vcp_5x_200, n);
    
    println!("\n=== Dynamic Fees ===");
    run_test("dyn_price_dev", dynamic_price_dev, n);
    run_test("dyn_smooth", dynamic_smooth, n);
}
