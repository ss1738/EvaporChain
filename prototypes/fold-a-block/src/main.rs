mod benchmark;
mod circuit;
mod state;

use std::time::Instant;

use nova_snark::{
    nova::{CompressedSNARK, PublicParams, RecursiveSNARK},
    provider::{PallasEngine, VestaEngine},
    traits::{snark::RelaxedR1CSSNARKTrait, Engine},
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

use benchmark::BenchmarkReport;
use circuit::EvaporBlockCircuit;
use state::BlockWitness;

// Type aliases for the Nova proving system
type E1 = PallasEngine;
type E2 = VestaEngine;
type EE1 = nova_snark::provider::ipa_pc::EvaluationEngine<E1>;
type EE2 = nova_snark::provider::ipa_pc::EvaluationEngine<E2>;
type S1 = nova_snark::spartan::snark::RelaxedR1CSSNARK<E1, EE1>;
type S2 = nova_snark::spartan::snark::RelaxedR1CSSNARK<E2, EE2>;
type C = EvaporBlockCircuit<<E1 as Engine>::GE>;

const NUM_BLOCKS: usize = 1000;

fn main() {
    println!("=== EvaporChain Fold-a-Block Prototype ===");
    println!(
        "Config: {} accounts, {} objects, {} txs/block, {} blocks",
        state::NUM_ACCOUNTS,
        state::NUM_OBJECTS,
        state::NUM_TXS,
        NUM_BLOCKS
    );
    println!();

    // 1. Setup public parameters
    let dummy_circuit = C::default_circuit();
    println!("Setting up Nova public parameters...");
    let setup_start = Instant::now();
    let pp = PublicParams::<E1, E2, C>::setup(
        &dummy_circuit,
        &*S1::ck_floor(),
        &*S2::ck_floor(),
    )
    .expect("Failed to set up public parameters");
    let setup_time = setup_start.elapsed();
    println!("Setup time: {:.2?}", setup_time);

    let num_constraints = pp.num_constraints().0; // primary circuit constraints
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
    println!();

    // 3. Initialize IVC with the first block
    let z0 = vec![
        <E1 as Engine>::Scalar::from(0u64),  // initial state hash
        <E1 as Engine>::Scalar::from(0u64),  // initial epoch
    ];

    let first_circuit = C::new(block_witnesses[0].clone());
    let mut recursive_snark =
        RecursiveSNARK::<E1, E2, C>::new(&pp, &first_circuit, &z0)
            .expect("Failed to create RecursiveSNARK");

    // Prove the first step
    let start = Instant::now();
    recursive_snark
        .prove_step(&pp, &first_circuit)
        .expect("Failed to prove first step");
    let first_fold = start.elapsed();
    println!("Block 0: fold={:?}", first_fold);

    // 4. Fold remaining blocks
    let mut fold_times = vec![first_fold];

    for (i, witness) in block_witnesses.iter().enumerate().skip(1) {
        let circuit_i = C::new(witness.clone());

        let start = Instant::now();
        recursive_snark
            .prove_step(&pp, &circuit_i)
            .expect(&format!("Failed to prove step {}", i));
        let elapsed = start.elapsed();

        fold_times.push(elapsed);

        if i % 100 == 0 || i == NUM_BLOCKS - 1 {
            println!("Block {}: fold={:?}", i, elapsed);
        }
    }

    // 5. Verify recursive SNARK
    println!("\nVerifying recursive SNARK...");
    let verify_start = Instant::now();
    let verify_result = recursive_snark.verify(&pp, NUM_BLOCKS, &z0);
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

    // 6. Compress to succinct SNARK
    println!("\nCompressing to succinct SNARK...");
    let compress_start = Instant::now();
    let (pk, vk) =
        CompressedSNARK::<_, _, _, S1, S2>::setup(&pp).expect("Failed to setup CompressedSNARK");
    let compressed = CompressedSNARK::<_, _, _, S1, S2>::prove(&pp, &pk, &recursive_snark)
        .expect("Failed to compress SNARK");
    let compression_time = compress_start.elapsed();
    println!("Compression time: {:.2?}", compression_time);

    let proof_bytes = bincode::serialize(&compressed).expect("Failed to serialize proof");
    println!("Compressed proof size: {} bytes", proof_bytes.len());

    // 7. Verify compressed SNARK
    println!("\nVerifying compressed SNARK...");
    let cverify_start = Instant::now();
    compressed
        .verify(&vk, NUM_BLOCKS, &z0)
        .expect("Compressed SNARK verification failed");
    let compressed_verify_time = cverify_start.elapsed();
    println!(
        "Compressed verification: {:?} -- VALID",
        compressed_verify_time
    );

    // 8. Generate report
    let report = BenchmarkReport::from_fold_times(
        &fold_times,
        num_constraints,
        setup_time,
        recursive_verify_time,
        compression_time,
        compressed_verify_time,
        proof_bytes.len(),
        evaporated,
    );

    report.print_report();

    // Save JSON report
    let json = serde_json::to_string_pretty(&report).expect("Failed to serialize report");
    std::fs::write("benchmark_results.json", &json).expect("Failed to write benchmark results");
    println!("\nResults saved to benchmark_results.json");
}
