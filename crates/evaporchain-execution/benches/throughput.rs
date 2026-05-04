use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use evaporchain_execution::parallel::ParallelExecutor;
use evaporchain_execution::ExecutionEngine;
use evaporchain_state::db::{InMemoryStateDB, StateDB};
use evaporchain_types::{Account, Block, Transaction, TransferTx};

fn make_address(id: u64) -> [u8; 32] {
    let mut addr = [0u8; 32];
    addr[0..8].copy_from_slice(&id.to_le_bytes());
    addr
}

fn seed_accounts(db: &mut InMemoryStateDB, count: u64, balance: u64) {
    for i in 0..count {
        db.put_account(Account {
            address: make_address(i),
            balance,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        });
    }
}

fn make_transfer_tx(from: u64, to: u64, amount: u64, nonce: u64) -> Transaction {
    Transaction::Transfer(TransferTx {
        from: make_address(from),
        to: make_address(to),
        amount,
        nonce,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    })
}

fn make_block_with_transfers(height: u64, num_txs: usize, account_count: u64) -> Block {
    let mut txs = Vec::with_capacity(num_txs);
    for i in 0..num_txs {
        let from = (i as u64) % account_count;
        let to = ((i as u64) + 1) % account_count;
        txs.push(make_transfer_tx(from, to, 10, i as u64));
    }

    Block {
        number: height,
        epoch: height,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        transactions: txs,
        timestamp: 1000 + height,
        chain_id: String::new(),
        producer_id: Some(1),
        vrf_output: None,
        vrf_proof: None,
        data_root: None,
        da_row_roots: vec![],
        da_col_roots: vec![],
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
    }
}

fn bench_block_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_execution");

    for tx_count in [100, 500, 1000, 5000] {
        let account_count = (tx_count as u64) + 10;

        group.bench_with_input(
            BenchmarkId::new("parallel", tx_count),
            &tx_count,
            |b, &tx_count| {
                b.iter_with_setup(
                    || {
                        let mut db = InMemoryStateDB::new();
                        seed_accounts(&mut db, account_count, 1_000_000_000);
                        let executor = ParallelExecutor::new_for_test(10);
                        let block = make_block_with_transfers(1, tx_count, account_count);
                        (db, executor, block)
                    },
                    |(mut db, mut executor, block)| {
                        let result = executor.execute_block(&mut db, &block);
                        black_box(result)
                    },
                );
            },
        );
    }
    group.finish();
}

fn bench_parallel_vs_sequential(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_vs_sequential");
    let tx_count = 1000;
    let account_count = 2000u64;

    // Non-conflicting transactions (different account pairs)
    group.bench_function("non_conflicting_1000tx", |b| {
        b.iter_with_setup(
            || {
                let mut db = InMemoryStateDB::new();
                seed_accounts(&mut db, account_count, 1_000_000_000);
                let executor = ParallelExecutor::new_for_test(10);
                let block = make_block_with_transfers(1, tx_count, account_count);
                (db, executor, block)
            },
            |(mut db, mut executor, block)| {
                let result = executor.execute_block(&mut db, &block);
                black_box(result)
            },
        );
    });

    // Conflicting transactions (all touch same account)
    group.bench_function("conflicting_1000tx", |b| {
        b.iter_with_setup(
            || {
                let mut db = InMemoryStateDB::new();
                seed_accounts(&mut db, account_count, 1_000_000_000);
                let executor = ParallelExecutor::new_for_test(10);

                let mut txs = Vec::with_capacity(tx_count);
                for i in 0..tx_count {
                    // All from account 0 → forces serialization
                    txs.push(make_transfer_tx(0, (i as u64) + 1, 10, i as u64));
                }
                let block = Block {
                    number: 1,
                    epoch: 1,
                    parent_hash: [0u8; 32],
                    state_root: [0u8; 32],
                    transactions: txs,
                    timestamp: 1001,
                    chain_id: String::new(),
                    producer_id: Some(1),
                    vrf_output: None,
                    vrf_proof: None,
                    data_root: None,
                    da_row_roots: vec![],
                    da_col_roots: vec![],
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
                };
                (db, executor, block)
            },
            |(mut db, mut executor, block)| {
                let result = executor.execute_block(&mut db, &block);
                black_box(result)
            },
        );
    });

    group.finish();
}

fn bench_state_root_computation(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_root");

    for account_count in [100, 1000, 10000] {
        group.bench_with_input(
            BenchmarkId::new("compute", account_count),
            &account_count,
            |b, &count| {
                b.iter_with_setup(
                    || {
                        let mut db = InMemoryStateDB::new();
                        seed_accounts(&mut db, count as u64, 1_000_000);
                        db
                    },
                    |mut db| {
                        let root = db.compute_state_root();
                        black_box(root)
                    },
                );
            },
        );
    }
    group.finish();
}

fn bench_transaction_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput_tps");
    group.sample_size(10);

    for tx_count in [1000, 5000, 10000] {
        let account_count = (tx_count as u64) * 2 + 10;

        group.bench_with_input(
            BenchmarkId::new("end_to_end", tx_count),
            &tx_count,
            |b, &tx_count| {
                b.iter_with_setup(
                    || {
                        let mut db = InMemoryStateDB::new();
                        seed_accounts(&mut db, account_count, 1_000_000_000);
                        let executor = ParallelExecutor::new_for_test(10);
                        let block = make_block_with_transfers(1, tx_count, account_count);
                        (db, executor, block)
                    },
                    |(mut db, mut executor, block)| {
                        let result = executor.execute_block(&mut db, &block);
                        let _root = db.compute_state_root();
                        black_box(result)
                    },
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_block_execution,
    bench_parallel_vs_sequential,
    bench_state_root_computation,
    bench_transaction_throughput,
);
criterion_main!(benches);
