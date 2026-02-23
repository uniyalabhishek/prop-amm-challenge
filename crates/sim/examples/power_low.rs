use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_sim::runner;

const NANO: f64 = 1_000_000_000.0;

fn to_nano(v: f64) -> u64 {
    if v <= 0.0 || !v.is_finite() { return 0; }
    let s = (v * NANO).floor();
    if s >= u64::MAX as f64 { u64::MAX } else { s as u64 }
}

// Power curve with α < 1: output = ro * (1 - (1 + γ*input/(α*ri))^(-α))
// Same marginal as CP at zero input
// More concave than CP at origin (concavity = (α+1)/α × CP)
// But UNLIKE exp(α), approaches FULL reserves for large input (not capped)
// Key insight: exp(0.20) scored poorly because it caps at 20% of reserves
// Power(0.20) has similar concavity but reaches full reserves → better retail depth
fn power_swap(data: &[u8], fee_bps: f64, alpha: f64) -> u64 {
    if data.len() < 25 { return 0; }
    let side = data[0];
    let input = u64::from_le_bytes(data[1..9].try_into().unwrap()) as f64 / NANO;
    let rx = u64::from_le_bytes(data[9..17].try_into().unwrap()) as f64 / NANO;
    let ry = u64::from_le_bytes(data[17..25].try_into().unwrap()) as f64 / NANO;
    if input <= 0.0 || rx <= 0.0 || ry <= 0.0 { return 0; }
    let gamma = (10000.0 - fee_bps) / 10000.0;
    let (ri, ro) = match side { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    
    let u = gamma * input / (alpha * ri);
    let base = (1.0 + u).powf(-alpha);
    let output = ro * (1.0 - base);
    
    if output <= 0.0 || !output.is_finite() { return 0; }
    to_nano(output.min(ro * 0.999))
}

// Exp baseline for comparison
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
    to_nano(output.min(ro * 0.999))
}

// Power curves with α < 1
fn p010_50(d: &[u8]) -> u64 { power_swap(d, 50.0, 0.10) }
fn p010_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.10) }
fn p010_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.10) }
fn p015_50(d: &[u8]) -> u64 { power_swap(d, 50.0, 0.15) }
fn p015_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.15) }
fn p015_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.15) }
fn p020_45(d: &[u8]) -> u64 { power_swap(d, 45.0, 0.20) }
fn p020_50(d: &[u8]) -> u64 { power_swap(d, 50.0, 0.20) }
fn p020_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.20) }
fn p020_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.20) }
fn p025_50(d: &[u8]) -> u64 { power_swap(d, 50.0, 0.25) }
fn p025_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.25) }
fn p025_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.25) }
fn p030_50(d: &[u8]) -> u64 { power_swap(d, 50.0, 0.30) }
fn p030_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.30) }
fn p030_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.30) }
fn p035_50(d: &[u8]) -> u64 { power_swap(d, 50.0, 0.35) }
fn p035_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.35) }
fn p035_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.35) }
fn p040_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.40) }
fn p040_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.40) }
fn p050_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.50) }
fn p050_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.50) }
fn p050_65(d: &[u8]) -> u64 { power_swap(d, 65.0, 0.50) }
fn p060_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.60) }
fn p060_65(d: &[u8]) -> u64 { power_swap(d, 65.0, 0.60) }
fn p070_65(d: &[u8]) -> u64 { power_swap(d, 65.0, 0.70) }
fn p080_65(d: &[u8]) -> u64 { power_swap(d, 65.0, 0.80) }

// Exp baselines
fn e030_55(d: &[u8]) -> u64 { exp_swap(d, 55.0, 0.30) }

fn run(name: &str, f: fn(&[u8]) -> u64, n: u32) {
    let r = runner::run_default_batch_native(
        f, None, normalizer_swap, Some(normalizer_after_swap), n, 10_000, None).unwrap();
    println!("{:>20}: avg={:>8.2}", name, r.avg_edge());
}

fn main() {
    let n = 1000u32;
    
    println!("=== Baseline ===");
    run("exp 0.30 55bp", e030_55, n);
    
    println!("\n=== Power α<1 (more concave, full reserve depth) ===");
    run("pow 0.10 50bp", p010_50, n);
    run("pow 0.10 55bp", p010_55, n);
    run("pow 0.10 60bp", p010_60, n);
    run("pow 0.15 50bp", p015_50, n);
    run("pow 0.15 55bp", p015_55, n);
    run("pow 0.15 60bp", p015_60, n);
    run("pow 0.20 45bp", p020_45, n);
    run("pow 0.20 50bp", p020_50, n);
    run("pow 0.20 55bp", p020_55, n);
    run("pow 0.20 60bp", p020_60, n);
    run("pow 0.25 50bp", p025_50, n);
    run("pow 0.25 55bp", p025_55, n);
    run("pow 0.25 60bp", p025_60, n);
    run("pow 0.30 50bp", p030_50, n);
    run("pow 0.30 55bp", p030_55, n);
    run("pow 0.30 60bp", p030_60, n);
    run("pow 0.35 50bp", p035_50, n);
    run("pow 0.35 55bp", p035_55, n);
    run("pow 0.35 60bp", p035_60, n);
    run("pow 0.40 55bp", p040_55, n);
    run("pow 0.40 60bp", p040_60, n);
    run("pow 0.50 55bp", p050_55, n);
    run("pow 0.50 60bp", p050_60, n);
    run("pow 0.50 65bp", p050_65, n);
    run("pow 0.60 60bp", p060_60, n);
    run("pow 0.60 65bp", p060_65, n);
    run("pow 0.70 65bp", p070_65, n);
    run("pow 0.80 65bp", p080_65, n);
}
