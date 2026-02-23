use prop_amm_shared::normalizer::compute_swap as normalizer_swap;
use prop_amm_shared::normalizer::after_swap as normalizer_after_swap;
use prop_amm_sim::runner;

const NANO: f64 = 1_000_000_000.0;

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
    let output = ro * (1.0 - (1.0 + u).powf(-alpha));
    if output <= 0.0 || !output.is_finite() { return 0; }
    let s = (output.min(ro * 0.999) * NANO).floor();
    if s <= 0.0 || s >= u64::MAX as f64 { 0 } else { s as u64 }
}

fn p035_50(d: &[u8]) -> u64 { power_swap(d, 50.0, 0.35) }
fn p035_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.35) }
fn p038_52(d: &[u8]) -> u64 { power_swap(d, 52.0, 0.38) }
fn p038_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.38) }
fn p040_52(d: &[u8]) -> u64 { power_swap(d, 52.0, 0.40) }
fn p040_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.40) }
fn p040_58(d: &[u8]) -> u64 { power_swap(d, 58.0, 0.40) }
fn p042_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.42) }
fn p042_58(d: &[u8]) -> u64 { power_swap(d, 58.0, 0.42) }
fn p045_55(d: &[u8]) -> u64 { power_swap(d, 55.0, 0.45) }
fn p045_58(d: &[u8]) -> u64 { power_swap(d, 58.0, 0.45) }
fn p045_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.45) }
fn p048_58(d: &[u8]) -> u64 { power_swap(d, 58.0, 0.48) }
fn p048_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.48) }
fn p050_58(d: &[u8]) -> u64 { power_swap(d, 58.0, 0.50) }
fn p050_60(d: &[u8]) -> u64 { power_swap(d, 60.0, 0.50) }
fn p050_62(d: &[u8]) -> u64 { power_swap(d, 62.0, 0.50) }

fn run(name: &str, f: fn(&[u8]) -> u64) {
    let r = runner::run_default_batch_native(
        f, None, normalizer_swap, Some(normalizer_after_swap), 1000, 10_000, None).unwrap();
    println!("{:>20}: avg={:>8.2}", name, r.avg_edge());
}

fn main() {
    println!("=== Power curve fine sweep ===");
    run("p0.35 50bp", p035_50);
    run("p0.35 55bp", p035_55);
    run("p0.38 52bp", p038_52);
    run("p0.38 55bp", p038_55);
    run("p0.40 52bp", p040_52);
    run("p0.40 55bp", p040_55);
    run("p0.40 58bp", p040_58);
    run("p0.42 55bp", p042_55);
    run("p0.42 58bp", p042_58);
    run("p0.45 55bp", p045_55);
    run("p0.45 58bp", p045_58);
    run("p0.45 60bp", p045_60);
    run("p0.48 58bp", p048_58);
    run("p0.48 60bp", p048_60);
    run("p0.50 58bp", p050_58);
    run("p0.50 60bp", p050_60);
    run("p0.50 62bp", p050_62);
}
