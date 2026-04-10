//! EvaporChain Benchmark Tool
//!
//! Measures TPS, block execution time, decay engine throughput,
//! and proof generation speed.

use evaporchain_execution::ExecutionEngine;
use evaporchain_execution::parallel::ParallelExecutor;
use evaporchain_state::db::{InMemoryStateDB, StateDB};
use evaporchain_types::{
    Account, Block, CreateObjectTx, RefreshTx, StateObject, Transaction, TransferTx,
};
use std::time::Instant;

/// Run all benchmarks and print results.
pub fn run_benchmarks() {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           EvaporChain Performance Benchmarks            ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!();

    let tps = bench_transaction_throughput();
    let block_exec = bench_block_execution();
    let decay = bench_decay_engine();
    let obj_creation = bench_object_creation();
    let refresh = bench_refresh_throughput();

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                    Summary                              ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  Transfer TPS:          {:>10.0} tx/s               ║", tps);
    println!("║  Block execution:       {:>10.2} ms/block            ║", block_exec);
    println!("║  Decay engine:          {:>10.0} objects/s           ║", decay);
    println!("║  Object creation:       {:>10.0} objects/s           ║", obj_creation);
    println!("║  Refresh throughput:    {:>10.0} refreshes/s         ║", refresh);
    println!("╚══════════════════════════════════════════════════════════╝");
}

/// Benchmark: raw transfer transaction throughput.
fn bench_transaction_throughput() -> f64 {
    print!("  [1/5] Transfer throughput ...          ");
    let num_txs = 10_000usize;
    let mut db = InMemoryStateDB::new();

    // Seed accounts
    for i in 0..100u8 {
        let mut addr = [0u8; 32];
        addr[0] = i;
        db.put_account(Account {
            address: addr,
            balance: 1_000_000_000,
            nonce: 0,
        });
    }

    // Build transactions
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
            })
        })
        .collect();

    let block = Block {
        number: 1,
        epoch: 1,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        transactions: txs,
        timestamp: 0,
        producer_id: None,
        vrf_output: None,
        vrf_proof: None,
            data_root: None,
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
    };

    let mut executor = ParallelExecutor::new(5);
    let start = Instant::now();
    let _ = executor.execute_block(&mut db, &block);
    let elapsed = start.elapsed();

    let tps = num_txs as f64 / elapsed.as_secs_f64();
    println!("{:>10.0} tx/s  ({:.1}ms)", tps, elapsed.as_secs_f64() * 1000.0);
    tps
}

/// Benchmark: full block execution (mixed transactions).
fn bench_block_execution() -> f64 {
    print!("  [2/5] Block execution time ...         ");
    let num_blocks = 100;
    let txs_per_block = 50;
    let mut db = InMemoryStateDB::new();

    // Seed accounts
    for i in 0..10u8 {
        let mut addr = [0u8; 32];
        addr[0] = i;
        db.put_account(Account {
            address: addr,
            balance: 1_000_000_000,
            nonce: 0,
        });
    }

    let mut executor = ParallelExecutor::new(5);
    let start = Instant::now();

    for block_num in 0..num_blocks {
        let txs: Vec<Transaction> = (0..txs_per_block)
            .map(|i| {
                let mut from = [0u8; 32];
                from[0] = (i % 10) as u8;
                let mut to = [0u8; 32];
                to[0] = ((i + 1) % 10) as u8;
                Transaction::Transfer(TransferTx {
                    from,
                    to,
                    amount: 1,
                    nonce: (block_num * txs_per_block / 10 + i / 10) as u64,
                    signature: None,
                    public_key: None,
                })
            })
            .collect();

        let block = Block {
            number: block_num as u64 + 1,
            epoch: block_num as u64 + 1,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: txs,
            timestamp: 0,
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
        };

        let _ = executor.execute_block(&mut db, &block);
    }

    let elapsed = start.elapsed();
    let ms_per_block = elapsed.as_secs_f64() * 1000.0 / num_blocks as f64;
    println!("{:>10.2} ms/block  ({} blocks)", ms_per_block, num_blocks);
    ms_per_block
}

/// Benchmark: decay engine — how fast we compute energy decay for objects.
fn bench_decay_engine() -> f64 {
    print!("  [3/5] Decay engine throughput ...       ");
    let num_objects = 100_000;

    let objects: Vec<StateObject> = (0..num_objects)
        .map(|i| {
            let mut id = [0u8; 32];
            id[0..8].copy_from_slice(&(i as u64).to_le_bytes());
            StateObject {
                id,
                owner: [1u8; 32],
                energy: 10_000 + (i as u64 % 50_000),
                half_life: 100 + (i as u64 % 200),
                created_at: 0,
                last_refreshed: 0,
                state: evaporchain_types::ObjectState::Active,
                grace_epoch: None,
                data: vec![0u8; 64],
            }
        })
        .collect();

    let current_epoch = 500;
    let start = Instant::now();

    let mut evaporated = 0u64;
    for obj in &objects {
        let energy = obj.energy_at(current_epoch);
        if energy == 0 {
            evaporated += 1;
        }
    }

    let elapsed = start.elapsed();
    let ops_per_sec = num_objects as f64 / elapsed.as_secs_f64();
    println!(
        "{:>10.0} obj/s  ({} evaporated)",
        ops_per_sec, evaporated
    );
    ops_per_sec
}

/// Benchmark: object creation throughput.
fn bench_object_creation() -> f64 {
    print!("  [4/5] Object creation throughput ...    ");
    let num_objects = 5_000;
    let mut db = InMemoryStateDB::new();

    // Seed creator account
    let creator = [42u8; 32];
    db.put_account(Account {
        address: creator,
        balance: 1_000_000_000,
        nonce: 0,
    });

    let txs: Vec<Transaction> = (0..num_objects)
        .map(|i| {
            let mut id = [0u8; 32];
            id[0..8].copy_from_slice(&(i as u64).to_le_bytes());
            Transaction::CreateObject(CreateObjectTx {
                creator,
                object_id: id,
                energy: 10_000,
                half_life: 100,
                data: vec![0u8; 32],
                signature: None,
                public_key: None,
            })
        })
        .collect();

    let block = Block {
        number: 1,
        epoch: 1,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        transactions: txs,
        timestamp: 0,
        producer_id: None,
        vrf_output: None,
        vrf_proof: None,
            data_root: None,
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
    };

    let mut executor = ParallelExecutor::new(5);
    let start = Instant::now();
    let _ = executor.execute_block(&mut db, &block);
    let elapsed = start.elapsed();

    let ops_per_sec = num_objects as f64 / elapsed.as_secs_f64();
    println!("{:>10.0} obj/s  ({:.1}ms)", ops_per_sec, elapsed.as_secs_f64() * 1000.0);
    ops_per_sec
}

/// Benchmark: refresh (energy top-up) throughput.
fn bench_refresh_throughput() -> f64 {
    print!("  [5/5] Refresh throughput ...            ");
    let num_refreshes = 5_000;
    let mut db = InMemoryStateDB::new();

    // Seed account and objects
    let owner = [1u8; 32];
    db.put_account(Account {
        address: owner,
        balance: 1_000_000_000,
        nonce: 0,
    });

    for i in 0..num_refreshes {
        let mut id = [0u8; 32];
        id[0..8].copy_from_slice(&(i as u64).to_le_bytes());
        db.put_object(StateObject {
            id,
            owner,
            energy: 100,
            half_life: 100,
            created_at: 0,
            last_refreshed: 0,
            state: evaporchain_types::ObjectState::Active,
            grace_epoch: None,
            data: vec![],
        });
    }

    let txs: Vec<Transaction> = (0..num_refreshes)
        .map(|i| {
            let mut id = [0u8; 32];
            id[0..8].copy_from_slice(&(i as u64).to_le_bytes());
            Transaction::Refresh(RefreshTx {
                object_id: id,
                energy_deposit: 5000,
                signature: None,
                public_key: None,
            })
        })
        .collect();

    let block = Block {
        number: 2,
        epoch: 50,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        transactions: txs,
        timestamp: 0,
        producer_id: None,
        vrf_output: None,
        vrf_proof: None,
            data_root: None,
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
    };

    let mut executor = ParallelExecutor::new(5);
    let start = Instant::now();
    let _ = executor.execute_block(&mut db, &block);
    let elapsed = start.elapsed();

    let ops_per_sec = num_refreshes as f64 / elapsed.as_secs_f64();
    println!("{:>10.0} ref/s  ({:.1}ms)", ops_per_sec, elapsed.as_secs_f64() * 1000.0);
    ops_per_sec
}
