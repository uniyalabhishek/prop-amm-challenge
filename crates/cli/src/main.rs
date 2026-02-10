mod commands;
mod output;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "prop-amm", about = "Prop AMM Challenge CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build program (native for simulation, BPF for submission)
    Build {
        /// Path to the program crate directory
        path: String,
    },
    /// Validate a BPF .so program (convexity, monotonicity, CU)
    Validate {
        /// Path to the compiled BPF .so file
        so_path: String,
    },
    /// Run simulation batch (BPF submission runtime)
    Run {
        /// Path to submission program artifact (BPF .so preferred; native lib accepted to locate companion BPF)
        lib_path: String,
        /// Number of simulations
        #[arg(long, default_value = "1000")]
        simulations: u32,
        /// Number of steps per simulation
        #[arg(long, default_value = "10000")]
        steps: u32,
        /// Number of parallel workers (0 = auto)
        #[arg(long, default_value = "0")]
        workers: usize,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build { path } => commands::build::run(&path),
        Commands::Validate { so_path } => commands::validate::run(&so_path),
        Commands::Run {
            lib_path,
            simulations,
            steps,
            workers,
        } => commands::run::run(&lib_path, simulations, steps, workers),
    }
}
