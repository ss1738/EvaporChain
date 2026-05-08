use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use evaporchain_execution::parallel::ParallelExecutor;
use evaporchain_execution::ExecutionEngine;
use evaporchain_state::db::{InMemoryStateDB, StateDB};
use evaporchain_types::{Account, Block, Transaction, TransferTx};

fn seed_db(db: &mut InMemoryStateDB, n_accounts: u8) {
    for i in 0..n_accounts {
        let mut addr = [0u8; 32];
        addr[0] = i;
        db.put_account(Account {
            address: addr,
            balance: 1_000_000_000,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
    }
}

fn make_transfer_block(num_txs: usize, block_num: u64) -> Block {
    let txs: Vec<Transaction> = (0..num_txs)
        .map(|i| {
            let mut from = [0u8; 32];
            from[0] = (i % 100) as u8;
            let mut to = [0u8; 32];
            to[0] = ((i + 1) % 100) as u8;
            Transaction::Transfer(TransferTx {
                from,
                to,
                amount: 1,
                nonce: (i / 100) as u64,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })
        })
        .collect();

    Block {
        number: block_num,
        epoch: 1,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        transactions: txs,
        timestamp: 0,
        chain_id: String::new(),
        producer_id: None,
        vrf_output: None,
        vrf_proof: None,
        data_root: None,
        blob_commitments: vec![],
        da_certificate: None,
        commit_certificate: None,
        nova_proof: None,
        anchor_hash: None,
        state_function_commitment: None,
        oracle_state_root: None,
        shard_count: None,
        protocol_version: 0,
        state_root_version: 0,
        submit_epoch_hints: vec![],
        parents: vec![],
        post_state_root: None,
        da_row_roots: vec![],
        da_col_roots: vec![],
    }
}

fn bench_transfer_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("transfer_throughput");
    for size in [100, 1000, 10000] {
        group.bench_with_input(
            criterion::BenchmarkId::new("parallel", size),
            &size,
            |b, &size| {
                b.iter_batched(
                    || {
                        let mut db = InMemoryStateDB::new();
                        seed_db(&mut db, 100);
                        let block = make_transfer_block(size, 1);
                        let executor = ParallelExecutor::new(5);
                        (db, block, executor)
                    },
                    |(mut db, block, mut executor)| {
                        black_box(executor.execute_block(&mut db, &block))
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_block_execution(c: &mut Criterion) {
    c.bench_function("block_execution_50tx", |b| {
        b.iter_batched(
            || {
                let mut db = InMemoryStateDB::new();
                seed_db(&mut db, 10);
                let block = make_transfer_block(50, 1);
                let executor = ParallelExecutor::new(5);
                (db, block, executor)
            },
            |(mut db, block, mut executor)| black_box(executor.execute_block(&mut db, &block)),
            BatchSize::SmallInput,
        );
    });
}

fn bench_poseidon_hash(c: &mut Criterion) {
    use evaporchain_crypto::hash::HashEngine;
    let hasher = evaporchain_crypto::hash::PoseidonHasher;
    let data = vec![42u8; 64];

    c.bench_function("poseidon_hash_64b", |b| {
        b.iter(|| black_box(hasher.hash(&data)))
    });
}

fn bench_signature(c: &mut Criterion) {
    use evaporchain_crypto::signatures::{MlDsaKeypair, MlDsaVerifier, Signer, Verifier};
    let kp = MlDsaKeypair::generate();
    let msg = b"benchmark message for signing";

    let mut group = c.benchmark_group("ml_dsa");
    group.bench_function("sign", |b| b.iter(|| black_box(kp.sign(msg))));

    let sig = kp.sign(msg);
    let pk = kp.public_key_bytes();
    group.bench_function("verify", |b| {
        b.iter(|| black_box(MlDsaVerifier::verify(msg, &sig, &pk)))
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_transfer_throughput,
    bench_block_execution,
    bench_poseidon_hash,
    bench_signature,
);
criterion_main!(benches);
