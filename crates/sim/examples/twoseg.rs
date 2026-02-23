use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_sim::runner;

const NANO: f64 = 1_000_000_000.0;

fn to_nano(v: f64) -> u64 {
    if v <= 0.0 || !v.is_finite() { return 0; }
    let s = (v * NANO).floor();
    if s >= u64::MAX as f64 { u64::MAX } else { s as u64 }
}

fn parse(d: &[u8]) -> Option<(u8, f64, f64, f64)> {
    if d.len() < 25 { return None; }
    let s = d[0];
    let i = u64::from_le_bytes(d[1..9].try_into().unwrap()) as f64 / NANO;
    let x = u64::from_le_bytes(d[9..17].try_into().unwrap()) as f64 / NANO;
    let y = u64::from_le_bytes(d[17..25].try_into().unwrap()) as f64 / NANO;
    if i <= 0.0 || x <= 0.0 || y <= 0.0 { return None; }
    Some((s, i, x, y))
}

// Two-segment exp: low fee for small Y (retail), high concavity for large Y (arb)
// Segment 1 (Y ≤ T): exp(α₁) with fee₁
// Segment 2 (Y > T): exp(α₂) with continuous slope, α₂ < α₁
fn twoseg_swap(d: &[u8], fee1_bps: f64, alpha1: f64, threshold_frac: f64, alpha2: f64) -> u64 {
    let (side, input, rx, ry) = match parse(d) { Some(v) => v, None => return 0 };
    let gamma1 = (10000.0 - fee1_bps) / 10000.0;
    let (ri, ro) = match side { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    
    // Threshold in input units (fraction of reserve_in)
    let t = threshold_frac * ri;
    
    if input <= t {
        // Segment 1: standard exp with alpha1 and fee1
        let u = gamma1 * input / (ri * alpha1);
        let output = alpha1 * ro * (1.0 - (-u).exp());
        to_nano(output.min(ro * 0.999))
    } else {
        // Segment 1 output at threshold
        let u_t = gamma1 * t / (ri * alpha1);
        let out1 = alpha1 * ro * (1.0 - (-u_t).exp());
        let slope_t = (gamma1 * ro / ri) * (-u_t).exp(); // slope at T from segment 1
        
        // Segment 2: new exp starting from (T, out1) with slope = slope_t
        // output = out1 + A2 * (1 - exp(-B2 * (Y - T)))
        // where A2 * B2 = slope_t, and A2 = alpha2 * remaining_ro
        let remaining_ro = (ro - out1).max(0.001);
        let a2 = alpha2 * remaining_ro;
        if a2 <= 0.0 { return to_nano(out1.min(ro * 0.999)); }
        let b2 = slope_t / a2;
        if b2 <= 0.0 || !b2.is_finite() { return to_nano(out1.min(ro * 0.999)); }
        
        let dy = input - t;
        let out2 = a2 * (1.0 - (-b2 * dy).exp());
        let output = out1 + out2;
        to_nano(output.min(ro * 0.999))
    }
}

// Simple exp baseline
fn exp_baseline(d: &[u8]) -> u64 {
    let (side, input, rx, ry) = match parse(d) { Some(v) => v, None => return 0 };
    let gamma = 0.9945;
    let alpha = 0.30;
    let (ri, ro) = match side { 0 => (ry, rx), 1 => (rx, ry), _ => return 0 };
    let u = gamma * input / (ri * alpha);
    to_nano((alpha * ro * (1.0 - (-u).exp())).min(ro * 0.999))
}

// Two-segment variants with different parameters
fn ts_45_030_003_015(d: &[u8]) -> u64 { twoseg_swap(d, 45.0, 0.30, 0.003, 0.15) }
fn ts_45_030_005_015(d: &[u8]) -> u64 { twoseg_swap(d, 45.0, 0.30, 0.005, 0.15) }
fn ts_45_030_010_015(d: &[u8]) -> u64 { twoseg_swap(d, 45.0, 0.30, 0.010, 0.15) }
fn ts_45_030_003_010(d: &[u8]) -> u64 { twoseg_swap(d, 45.0, 0.30, 0.003, 0.10) }
fn ts_45_030_005_010(d: &[u8]) -> u64 { twoseg_swap(d, 45.0, 0.30, 0.005, 0.10) }
fn ts_45_030_010_010(d: &[u8]) -> u64 { twoseg_swap(d, 45.0, 0.30, 0.010, 0.10) }

fn ts_50_030_003_015(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.30, 0.003, 0.15) }
fn ts_50_030_005_015(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.30, 0.005, 0.15) }
fn ts_50_030_010_015(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.30, 0.010, 0.15) }
fn ts_50_030_003_010(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.30, 0.003, 0.10) }
fn ts_50_030_005_010(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.30, 0.005, 0.10) }
fn ts_50_030_005_020(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.30, 0.005, 0.20) }

fn ts_55_030_003_015(d: &[u8]) -> u64 { twoseg_swap(d, 55.0, 0.30, 0.003, 0.15) }
fn ts_55_030_005_015(d: &[u8]) -> u64 { twoseg_swap(d, 55.0, 0.30, 0.005, 0.15) }
fn ts_55_030_003_010(d: &[u8]) -> u64 { twoseg_swap(d, 55.0, 0.30, 0.003, 0.10) }
fn ts_55_030_005_010(d: &[u8]) -> u64 { twoseg_swap(d, 55.0, 0.30, 0.005, 0.10) }

fn ts_50_040_005_015(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.40, 0.005, 0.15) }
fn ts_50_040_005_020(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.40, 0.005, 0.20) }
fn ts_50_040_003_010(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.40, 0.003, 0.10) }
fn ts_50_040_003_015(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.40, 0.003, 0.15) }
fn ts_50_050_005_015(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.50, 0.005, 0.15) }
fn ts_50_050_005_020(d: &[u8]) -> u64 { twoseg_swap(d, 50.0, 0.50, 0.005, 0.20) }

fn run(name: &str, f: fn(&[u8]) -> u64, n: u32) {
    let r = runner::run_default_batch_native(
        f, None, normalizer_swap, Some(normalizer_after_swap), n, 10_000, None).unwrap();
    println!("{:>35}: avg={:>8.2}", name, r.avg_edge());
}

fn main() {
    let n = 1000u32;
    
    println!("=== Baseline ===");
    run("exp 0.30 55bp", exp_baseline, n);
    
    println!("\n=== Two-Segment: fee1=45bp ===");
    run("45bp a1=0.30 t=0.003 a2=0.15", ts_45_030_003_015, n);
    run("45bp a1=0.30 t=0.005 a2=0.15", ts_45_030_005_015, n);
    run("45bp a1=0.30 t=0.010 a2=0.15", ts_45_030_010_015, n);
    run("45bp a1=0.30 t=0.003 a2=0.10", ts_45_030_003_010, n);
    run("45bp a1=0.30 t=0.005 a2=0.10", ts_45_030_005_010, n);
    run("45bp a1=0.30 t=0.010 a2=0.10", ts_45_030_010_010, n);
    
    println!("\n=== Two-Segment: fee1=50bp ===");
    run("50bp a1=0.30 t=0.003 a2=0.15", ts_50_030_003_015, n);
    run("50bp a1=0.30 t=0.005 a2=0.15", ts_50_030_005_015, n);
    run("50bp a1=0.30 t=0.010 a2=0.15", ts_50_030_010_015, n);
    run("50bp a1=0.30 t=0.003 a2=0.10", ts_50_030_003_010, n);
    run("50bp a1=0.30 t=0.005 a2=0.10", ts_50_030_005_010, n);
    run("50bp a1=0.30 t=0.005 a2=0.20", ts_50_030_005_020, n);
    
    println!("\n=== Two-Segment: fee1=55bp ===");
    run("55bp a1=0.30 t=0.003 a2=0.15", ts_55_030_003_015, n);
    run("55bp a1=0.30 t=0.005 a2=0.15", ts_55_030_005_015, n);
    run("55bp a1=0.30 t=0.003 a2=0.10", ts_55_030_003_010, n);
    run("55bp a1=0.30 t=0.005 a2=0.10", ts_55_030_005_010, n);
    
    println!("\n=== Two-Segment: higher a1 ===");
    run("50bp a1=0.40 t=0.005 a2=0.15", ts_50_040_005_015, n);
    run("50bp a1=0.40 t=0.005 a2=0.20", ts_50_040_005_020, n);
    run("50bp a1=0.40 t=0.003 a2=0.10", ts_50_040_003_010, n);
    run("50bp a1=0.40 t=0.003 a2=0.15", ts_50_040_003_015, n);
    run("50bp a1=0.50 t=0.005 a2=0.15", ts_50_050_005_015, n);
    run("50bp a1=0.50 t=0.005 a2=0.20", ts_50_050_005_020, n);
}
