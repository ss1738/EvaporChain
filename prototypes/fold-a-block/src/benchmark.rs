use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub struct BenchmarkReport {
    pub engine: String,
    pub blocks_folded: usize,
    pub blocks_per_fold: usize,
    pub num_fold_steps: usize,
    pub num_constraints: usize,
    pub num_accounts: usize,
    pub num_objects: usize,
    pub num_txs: usize,
    pub setup_time_s: f64,
    pub avg_fold_ms: f64,
    pub min_fold_ms: f64,
    pub max_fold_ms: f64,
    pub median_fold_ms: f64,
    pub p95_fold_ms: f64,
    pub p99_fold_ms: f64,
    pub total_fold_s: f64,
    pub amortized_per_block_ms: f64,
    pub recursive_verify_ms: f64,
    pub compression_time_s: f64,
    pub compressed_verify_ms: f64,
    pub proof_size_bytes: usize,
    pub evaporated_objects: usize,
    pub verdict: String,
}

impl BenchmarkReport {
    pub fn from_fold_times(
        fold_times: &[Duration],
        num_constraints: usize,
        setup_time: Duration,
        recursive_verify_time: Duration,
        compression_time: Duration,
        compressed_verify_time: Duration,
        proof_size: usize,
        evaporated: usize,
        blocks_per_fold: usize,
        total_blocks: usize,
    ) -> Self {
        let mut sorted: Vec<f64> = fold_times.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let n = sorted.len();
        let avg = sorted.iter().sum::<f64>() / n as f64;
        let min = sorted[0];
        let max = sorted[n - 1];
        let median = sorted[n / 2];
        let p95 = sorted[(n as f64 * 0.95) as usize];
        let p99 = sorted[(n as f64 * 0.99) as usize];
        let total: f64 = fold_times.iter().map(|d| d.as_secs_f64()).sum();

        let amortized_per_block_ms = (total / total_blocks as f64) * 1000.0;

        // Verdict based on amortized per-block time
        let verdict = if amortized_per_block_ms < 10.0 {
            "PASS".to_string()
        } else if amortized_per_block_ms < 50.0 {
            "MARGINAL".to_string()
        } else {
            "FAIL".to_string()
        };

        Self {
            engine: "Bn256/Grumpkin + HyperKZG".to_string(),
            blocks_folded: total_blocks,
            blocks_per_fold,
            num_fold_steps: n,
            num_constraints,
            num_accounts: crate::state::NUM_ACCOUNTS,
            num_objects: crate::state::NUM_OBJECTS,
            num_txs: crate::state::NUM_TXS,
            setup_time_s: setup_time.as_secs_f64(),
            avg_fold_ms: avg,
            min_fold_ms: min,
            max_fold_ms: max,
            median_fold_ms: median,
            p95_fold_ms: p95,
            p99_fold_ms: p99,
            total_fold_s: total,
            amortized_per_block_ms,
            recursive_verify_ms: recursive_verify_time.as_secs_f64() * 1000.0,
            compression_time_s: compression_time.as_secs_f64(),
            compressed_verify_ms: compressed_verify_time.as_secs_f64() * 1000.0,
            proof_size_bytes: proof_size,
            evaporated_objects: evaporated,
            verdict,
        }
    }

    pub fn print_report(&self) {
        println!("\n{}", "=".repeat(60));
        println!("  EVAPORCHAIN FOLD-A-BLOCK BENCHMARK RESULTS (OPTIMIZED)");
        println!("{}\n", "=".repeat(60));

        println!("Engine: {}", self.engine);
        println!();

        println!("Circuit Configuration:");
        println!("  Accounts:         {}", self.num_accounts);
        println!("  Objects:          {}", self.num_objects);
        println!("  Txs per block:    {}", self.num_txs);
        println!("  Blocks per fold:  {}", self.blocks_per_fold);
        println!("  R1CS constraints: {}", self.num_constraints);
        println!();

        println!("Setup:");
        println!("  Public params:    {:.2}s", self.setup_time_s);
        println!();

        println!("Folding ({} blocks in {} fold steps):", self.blocks_folded, self.num_fold_steps);
        println!("  Avg per fold:     {:.3}ms", self.avg_fold_ms);
        println!("  Median per fold:  {:.3}ms", self.median_fold_ms);
        println!("  Min per fold:     {:.3}ms", self.min_fold_ms);
        println!("  Max per fold:     {:.3}ms", self.max_fold_ms);
        println!("  P95 per fold:     {:.3}ms", self.p95_fold_ms);
        println!("  P99 per fold:     {:.3}ms", self.p99_fold_ms);
        println!("  Total:            {:.2}s", self.total_fold_s);
        println!();

        println!("Amortized Performance:");
        println!("  Per-block time:   {:.3}ms", self.amortized_per_block_ms);
        println!();

        println!("Verification:");
        println!("  Recursive:        {:.2}ms", self.recursive_verify_ms);
        println!("  Compressed:       {:.2}ms", self.compressed_verify_ms);
        println!();

        println!("Compression:");
        println!("  Time:             {:.2}s", self.compression_time_s);
        println!("  Proof size:       {} bytes ({:.1} KB)", self.proof_size_bytes, self.proof_size_bytes as f64 / 1024.0);
        println!();

        println!("State Decay:");
        println!(
            "  Evaporated:       {}/{}",
            self.evaporated_objects, self.num_objects
        );
        println!();

        let ms = self.amortized_per_block_ms;
        match self.verdict.as_str() {
            "PASS" => {
                println!("VERDICT: PASS -- Amortized {:.1}ms/block < 10ms. EvaporChain is feasible.", ms);
            }
            "MARGINAL" => {
                println!("VERDICT: MARGINAL -- Amortized {:.1}ms/block. Feasible with further optimization.", ms);
            }
            _ => {
                println!("VERDICT: FAIL -- Amortized {:.1}ms/block exceeds 50ms threshold.", ms);
            }
        }
    }
}
