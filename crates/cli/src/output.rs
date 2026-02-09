use prop_amm_shared::result::BatchResult;
use std::time::Duration;

pub fn print_results(result: &BatchResult, elapsed: Duration) {
    println!("\n========================================");
    println!("  Simulations: {}", result.n_sims());
    println!("  Time:        {:.2}s", elapsed.as_secs_f64());
    println!("  Avg edge:    {:.2}", result.avg_edge());
    println!("  Total edge:  {:.2}", result.total_edge);
    println!("========================================");
}
