mod benchmark;
mod circuit;
mod state;

use std::time::Instant;

use nova_snark::{
    nova::{CompressedSNARK, PublicParams, RecursiveSNARK},
    provider::{Bn256EngineKZG, GrumpkinEngine},
    traits::{snark::RelaxedR1CSSNARKTrait, Engine},
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

use benchmark::BenchmarkReport;
use circuit::EvaporBlockCircuit;
use state::BlockWitness;

// Type aliases: Bn256/Grumpkin + HyperKZG (primary) + IPA (secondary)
type E1 = Bn256EngineKZG;
type E2 = GrumpkinEngine;
type EE1 = nova_snark::provider::hyperkzg::EvaluationEngine<E1>;
type EE2 = nova_snark::provider::ipa_pc::EvaluationEngine<E2>;
type S1 = nova_snark::spartan::snark::RelaxedR1CSSNARK<E1, EE1>;
type S2 = nova_snark::spartan::snark::RelaxedR1CSSNARK<E2, EE2>;
type C = EvaporBlockCircuit<<E1 as Engine>::GE>;

const NUM_BLOCKS: usize = 1000;
const BLOCKS_PER_FOLD: usize = 5;

fn main() {
    println!("=== EvaporChain Fold-a-Block Prototype (Optimized) ===");
    println!("Engine: Bn256/Grumpkin + HyperKZG");
    println!("Batching: {} blocks per fold step", BLOCKS_PER_FOLD);
    println!(
        "Config: {} accounts, {} objects, {} txs/block, {} blocks",
        state::NUM_ACCOUNTS,
        state::NUM_OBJECTS,
        state::NUM_TXS,
        NUM_BLOCKS
    );
    println!();

    // 1. Setup public parameters
    let dummy_witnesses: Vec<BlockWitness> = (0..BLOCKS_PER_FOLD)
        .map(|_| BlockWitness::genesis())
        .collect();
    let dummy_circuit = C::new_batched(dummy_witnesses);
    println!("Setting up Nova public parameters (HyperKZG)...");
    let setup_start = Instant::now();
    let pp = PublicParams::<E1, E2, C>::setup(
        &dummy_circuit,
        &*S1::ck_floor(),
        &*S2::ck_floor(),
    )
    .expect("Failed to set up public parameters");
    let setup_time = setup_start.elapsed();
    println!("Setup time: {:.2?}", setup_time);

    let num_constraints = pp.num_constraints().0;
    println!("R1CS constraints (primary): {}", num_constraints);
    println!("R1CS constraints (secondary): {}", pp.num_constraints().1);
    println!();

    // 2. Generate block sequence
    let mut rng = ChaCha20Rng::seed_from_u64(42);
    let mut current_state = BlockWitness::genesis();
    let mut block_witnesses: Vec<BlockWitness> = Vec::with_capacity(NUM_BLOCKS);

    for _ in 0..NUM_BLOCKS {
        let next = current_state.random_next(&mut rng);
        block_witnesses.push(next.clone());
        current_state = next;
    }

    let evaporated = current_state.evaporated_count();
    println!(
        "Generated {} blocks. Evaporated objects after {} epochs: {}/{}",
        NUM_BLOCKS,
        current_state.epoch,
        evaporated,
        state::NUM_OBJECTS,
    );

    // 3. Batch blocks into fold steps (pad last batch to uniform size)
    let batches: Vec<Vec<BlockWitness>> = block_witnesses
        .chunks(BLOCKS_PER_FOLD)
        .map(|chunk| {
            let mut batch = chunk.to_vec();
            // Pad with copies of the last block to keep circuit shape uniform
            while batch.len() < BLOCKS_PER_FOLD {
                batch.push(batch.last().unwrap().clone());
            }
            batch
        })
        .collect();
    let num_folds = batches.len();
    println!(
        "Batched into {} fold steps ({} blocks per fold)",
        num_folds, BLOCKS_PER_FOLD
    );
    println!();

    // 4. Initialize IVC with the first batch
    let z0 = vec![
        <E1 as Engine>::Scalar::from(0u64),
        <E1 as Engine>::Scalar::from(0u64),
    ];

    let first_circuit = C::new_batched(batches[0].clone());
    let mut recursive_snark =
        RecursiveSNARK::<E1, E2, C>::new(&pp, &first_circuit, &z0)
            .expect("Failed to create RecursiveSNARK");

    let start = Instant::now();
    recursive_snark
        .prove_step(&pp, &first_circuit)
        .expect("Failed to prove first step");
    let first_fold = start.elapsed();
    println!("Fold 0 (blocks 0-{}): {:?}", BLOCKS_PER_FOLD - 1, first_fold);

    // 5. Fold remaining batches
    let mut fold_times = vec![first_fold];

    for (i, batch) in batches.iter().enumerate().skip(1) {
        let circuit_i = C::new_batched(batch.clone());

        let start = Instant::now();
        recursive_snark
            .prove_step(&pp, &circuit_i)
            .unwrap_or_else(|_| panic!("Failed to prove fold step {}", i));
        let elapsed = start.elapsed();

        fold_times.push(elapsed);

        if i % 50 == 0 || i == num_folds - 1 {
            let block_start = i * BLOCKS_PER_FOLD;
            let block_end = (block_start + BLOCKS_PER_FOLD).min(NUM_BLOCKS) - 1;
            println!("Fold {} (blocks {}-{}): {:?}", i, block_start, block_end, elapsed);
        }
    }

    // 6. Verify recursive SNARK
    println!("\nVerifying recursive SNARK...");
    let verify_start = Instant::now();
    let verify_result = recursive_snark.verify(&pp, num_folds, &z0);
    let recursive_verify_time = verify_start.elapsed();

    match &verify_result {
        Ok(z_final) => {
            println!("Recursive verification: {:?} -- VALID", recursive_verify_time);
            println!("Final state: epoch={:?}", z_final[1]);
        }
        Err(e) => {
            println!("Recursive verification FAILED: {:?}", e);
            println!("Continuing to compression anyway...");
        }
    }

    // 7. Compress to succinct SNARK (HyperKZG)
    println!("\nCompressing to succinct SNARK (HyperKZG)...");
    let compress_start = Instant::now();
    let (pk, vk) =
        CompressedSNARK::<_, _, _, S1, S2>::setup(&pp).expect("Failed to setup CompressedSNARK");
    let compressed = CompressedSNARK::<_, _, _, S1, S2>::prove(&pp, &pk, &recursive_snark)
        .expect("Failed to compress SNARK");
    let compression_time = compress_start.elapsed();
    println!("Compression time: {:.2?}", compression_time);

    let proof_bytes = bincode::serialize(&compressed).expect("Failed to serialize proof");
    println!("Compressed proof size: {} bytes", proof_bytes.len());

    // 8. Verify compressed SNARK
    println!("\nVerifying compressed SNARK...");
    let cverify_start = Instant::now();
    compressed
        .verify(&vk, num_folds, &z0)
        .expect("Compressed SNARK verification failed");
    let compressed_verify_time = cverify_start.elapsed();
    println!(
        "Compressed verification: {:?} -- VALID",
        compressed_verify_time
    );

    // 9. Generate report
    // Compute amortized per-block times from fold times
    let total_fold_time: f64 = fold_times.iter().map(|d| d.as_secs_f64()).sum();
    let amortized_per_block_ms = (total_fold_time / NUM_BLOCKS as f64) * 1000.0;
    println!("\nAmortized per-block fold time: {:.3}ms", amortized_per_block_ms);

    let report = BenchmarkReport::from_fold_times(
        &fold_times,
        num_constraints,
        setup_time,
        recursive_verify_time,
        compression_time,
        compressed_verify_time,
        proof_bytes.len(),
        evaporated,
        BLOCKS_PER_FOLD,
        NUM_BLOCKS,
    );

    report.print_report();

    // Save JSON report
    let json = serde_json::to_string_pretty(&report).expect("Failed to serialize report");
    std::fs::write("benchmark_results.json", &json).expect("Failed to write benchmark results");
    println!("\nResults saved to benchmark_results.json");
}
