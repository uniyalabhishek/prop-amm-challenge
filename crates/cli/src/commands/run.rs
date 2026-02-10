use std::path::{Path, PathBuf};

use prop_amm_executor::BpfProgram;
use prop_amm_shared::normalizer::{
    after_swap as normalizer_after_swap_fn, compute_swap as normalizer_swap,
};
use prop_amm_sim::runner;

use crate::output;

pub fn run(program_path: &str, simulations: u32, steps: u32, workers: usize) -> anyhow::Result<()> {
    let submission_program = load_submission_bpf(program_path)?;
    let n_workers = if workers == 0 { None } else { Some(workers) };

    println!(
        "Running {} simulations ({} steps each) with BPF submission runtime...",
        simulations, steps,
    );

    let start = std::time::Instant::now();
    let result = runner::run_default_batch_mixed(
        submission_program,
        normalizer_swap,
        Some(normalizer_after_swap_fn),
        simulations,
        steps,
        n_workers,
    )?;
    let elapsed = start.elapsed();

    output::print_results(&result, elapsed);
    Ok(())
}

fn load_submission_bpf(path: &str) -> anyhow::Result<BpfProgram> {
    let provided = Path::new(path);
    if provided.exists() {
        let bytes = std::fs::read(provided)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", provided.display(), e))?;
        if let Ok(program) = BpfProgram::load(&bytes) {
            return Ok(program);
        }
    }

    let bpf_path = find_companion_bpf(provided).ok_or_else(|| {
        anyhow::anyhow!(
            "Path {} is not a loadable BPF program and no companion BPF artifact was found. \
             Pass the BPF .so path or build with `prop-amm build <crate-dir>` and pass the native library path.",
            path
        )
    })?;

    let bytes = std::fs::read(&bpf_path)
        .map_err(|e| anyhow::anyhow!("Failed to read BPF program {}: {}", bpf_path.display(), e))?;
    BpfProgram::load(&bytes)
        .map_err(|e| anyhow::anyhow!("Failed to load BPF program {}: {}", bpf_path.display(), e))
}

fn find_companion_bpf(native_lib: &Path) -> Option<PathBuf> {
    let release_dir = native_lib.parent()?;
    if release_dir.file_name()?.to_str()? != "release" {
        return None;
    }
    let target_dir = release_dir.parent()?;
    if target_dir.file_name()?.to_str()? != "target" {
        return None;
    }
    let crate_dir = target_dir.parent()?;
    let cargo_toml = crate_dir.join("Cargo.toml");
    let pkg_name = read_package_name(&cargo_toml)?;
    let so_name = format!("{}.so", pkg_name.replace('-', "_"));
    let bpf_path = crate_dir.join("target").join("deploy").join(so_name);
    if bpf_path.exists() {
        Some(bpf_path)
    } else {
        None
    }
}

fn read_package_name(cargo_toml: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(cargo_toml).ok()?;
    let mut in_package = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            if !rest.starts_with('=') {
                continue;
            }
            let mut value = rest[1..].trim();
            if let Some(comment_idx) = value.find('#') {
                value = value[..comment_idx].trim();
            }
            if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                return Some(value[1..value.len() - 1].to_string());
            }
        }
    }
    None
}
