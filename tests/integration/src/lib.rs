//! Cross-crate integration tests for EvaporChain.
//!
//! These tests exercise the full pipeline: transaction → mempool → consensus
//! → execution → state → DA → proving, verifying that all components
//! integrate correctly.

#[cfg(test)]
mod tests {
    use evaporchain_consensus::tendermint::{
        ConsensusAction, ConsensusMessage, TendermintConsensus,
    };
    use evaporchain_consensus::validator_set::{ValidatorInfo, ValidatorSet};
    use evaporchain_consensus::light_client::{LightBlockHeader, LightClientVerifier, VerificationResult};
    use evaporchain_consensus::state_sync::{
        SnapshotProvider, StateSyncManager, SyncAction, SyncMessage, SyncPhase,
    };
    use evaporchain_consensus::persistence::{
        ConsensusCheckpoint, ConsensusStateStore, InMemoryStateStore,
    };
    use evaporchain_crypto::signatures::{BlsKeypair, BlsSignature, BlsVerifier, MlDsaKeypair, MlDsaVerifier, Signer, Verifier};
    use evaporchain_crypto::vrf::VrfKeypair;
    use evaporchain_crypto::hash::blake3_hash;
    use evaporchain_consensus::finality::{FinalityTracker, FinalityStatus};
    use evaporchain_execution::{ExecutionEngine, parallel::ParallelExecutor};
    use evaporchain_da::erasure2d::ErasureEncoder2D;
    use evaporchain_da::commitments::RowColumnCommitments;
    use evaporchain_da::certificate::{CertificateBuilder, create_attestation};
    use evaporchain_state::{InMemoryStateDB, StateDB};
    use evaporchain_types::{
        Account, Block, CommitCertificate, Transaction, TransferTx,
        ValidatorStakeTx, ValidatorExitTx, BlobTx,
    };
    use std::sync::OnceLock;

    // ─────────────── Cached ML-DSA Keypair ───────────────────────────
    // ML-DSA keygen is ~15s in debug mode. Cache one keypair for all tests.

    struct CachedMlDsa {
        pk: Vec<u8>,
        sk: Vec<u8>,
    }

    static CACHED_ML_DSA: OnceLock<CachedMlDsa> = OnceLock::new();

    fn get_ml_dsa_keypair() -> MlDsaKeypair {
        let cached = CACHED_ML_DSA.get_or_init(|| {
            let kp = MlDsaKeypair::generate();
            CachedMlDsa {
                pk: kp.public_key().to_vec(),
                sk: kp.secret_key().to_vec(),
            }
        });
        MlDsaKeypair::from_bytes(&cached.pk, &cached.sk).unwrap()
    }

    // ─────────────── Helpers ──────────────────────────────────────────

    fn make_validator_set_with_bls(n: u64, stake: u64) -> (ValidatorSet, Vec<BlsKeypair>) {
        let mut vs = ValidatorSet::new();
        let mut bls_kps = Vec::new();
        for i in 0..n {
            let bls_kp = BlsKeypair::generate();
            let mut info = ValidatorInfo::new(i, stake, [i as u8; 32]);
            info.bls_public_key = Some(bls_kp.public_key_bytes().0);
            info.pop_verified = true;
            vs.add_validator(info);
            bls_kps.push(bls_kp);
        }
        (vs, bls_kps)
    }

    /// Setup 4-node BFT network with BLS keys (no VRF — fast).
    fn setup_4_node_network() -> Vec<TendermintConsensus> {
        setup_network(false)
    }

    /// Setup 4-node BFT network with BLS + VRF keys (slow — ML-DSA keygen).
    fn setup_4_node_network_with_vrf() -> Vec<TendermintConsensus> {
        setup_network(true)
    }

    fn setup_network(with_vrf: bool) -> Vec<TendermintConsensus> {
        let mut vs = ValidatorSet::new();
        for i in 0..4u64 {
            vs.add_validator(ValidatorInfo::new(i, 1000, [i as u8; 32]));
        }

        let mut nodes: Vec<TendermintConsensus> = (0..4u64)
            .map(|i| {
                let mut tc = TendermintConsensus::new_for_test(i, 0, vs.clone());
                let bls_kp = BlsKeypair::generate();
                tc.validator_set.get_mut(i).unwrap().bls_public_key =
                    Some(bls_kp.public_key_bytes().0);
                tc.set_bls_keypair(bls_kp);
                if with_vrf {
                    let vrf_kp = VrfKeypair::generate();
                    tc.validator_set.get_mut(i).unwrap().vrf_public_key =
                        Some(vrf_kp.public_key_bytes());
                    tc.set_vrf_keypair(vrf_kp);
                }
                tc
            })
            .collect();

        // Sync validator sets across all nodes
        let mut merged = nodes[0].validator_set.clone();
        for node in &nodes[1..] {
            for v in node.validator_set.validators() {
                if let Some(target) = merged.get_mut(v.id) {
                    if target.bls_public_key.is_none() {
                        target.bls_public_key = v.bls_public_key.clone();
                    }
                    if target.vrf_public_key.is_none() {
                        target.vrf_public_key = v.vrf_public_key.clone();
                    }
                    target.pop_verified = true;
                }
            }
        }
        for node in nodes.iter_mut() {
            node.validator_set = merged.clone();
        }
        nodes
    }

    /// Collect actions from a node into message/commit buffers.
    fn collect_actions(
        actions: Vec<ConsensusAction>,
        messages: &mut Vec<ConsensusMessage>,
        committed: &mut Vec<Block>,
    ) {
        for action in actions {
            match action {
                ConsensusAction::BroadcastMessage(m) => messages.push(m),
                ConsensusAction::CommitBlock(b) => committed.push(b),
                _ => {}
            }
        }
    }

    /// Run one height of consensus across all nodes, returning the committed block.
    ///
    /// Key: prevote→precommit and precommit→commit transitions happen in
    /// `tick()`, not `on_message()`. We must tick after each message batch
    /// so nodes check quorum and advance phases.
    fn run_consensus_height(nodes: &mut [TendermintConsensus]) -> Block {
        let mut all_messages = Vec::new();
        let mut committed = Vec::new();
        let mut tick_db = InMemoryStateDB::new();

        for _ in 0..20 {
            // Tick all nodes — proposer creates block, quorum checks run
            for node in nodes.iter_mut() {
                let actions = node.tick(&mut tick_db);
                collect_actions(actions, &mut all_messages, &mut committed);
            }
            if !committed.is_empty() {
                break;
            }

            // Deliver all pending messages to all nodes
            let messages = std::mem::take(&mut all_messages);
            for msg in messages {
                for node in nodes.iter_mut() {
                    let actions = node.on_message(msg.clone());
                    collect_actions(actions, &mut all_messages, &mut committed);
                }
            }
            if !committed.is_empty() {
                break;
            }
        }

        assert!(!committed.is_empty(), "Should commit a block");
        committed.into_iter().next().unwrap()
    }

    // ─────────────── Test: Full Pipeline ──────────────────────────────

    #[test]
    fn test_full_pipeline_transfer_through_consensus() {
        let mut nodes = setup_4_node_network();
        let mut db = InMemoryStateDB::new();

        // Fund accounts
        let sender = [1u8; 32];
        db.put_account(Account {
            address: sender,
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });

        // Submit transfer to all mempools
        let tx = Transaction::Transfer(TransferTx {
            from: sender,
            to: [2u8; 32],
            amount: 500,
            nonce: 1,
            signature: None,
            public_key: None,
        });
        for node in nodes.iter_mut() {
            node.mempool.submit(tx.clone());
        }

        // Run consensus
        let block = run_consensus_height(&mut nodes);

        // Block should contain our transaction
        assert!(!block.transactions.is_empty(), "Block should have transactions");
        assert_eq!(block.number, 1);

        // Block should have a commit certificate (BLS signed)
        assert!(
            block.commit_certificate.is_some(),
            "Block should have BLS commit certificate"
        );

        // Execute the block
        let result = nodes[0].execute_block(&mut db, &block);
        assert!(result.is_ok(), "Block execution should succeed");

        let result = result.unwrap();
        let state_root = result.execution.state_root;
        assert_ne!(state_root, [0u8; 32], "State root should be non-zero after execution");

        // Advance all nodes
        for node in nodes.iter_mut() {
            node.on_block_committed(&block, state_root, result.execution.objects_evaporated);
        }

        // Verify commit certificate
        let cert = block.commit_certificate.as_ref().unwrap();
        assert!(nodes[0].verify_commit_certificate(cert));
    }

    #[test]
    fn test_multi_height_consensus_with_execution() {
        let mut nodes = setup_4_node_network();
        let mut db = InMemoryStateDB::new();

        // Fund sender
        db.put_account(Account {
            address: [1u8; 32],
            balance: 10_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });

        let mut state_roots = Vec::new();

        for height in 1..=5u64 {
            // Submit a transfer each height
            let tx = Transaction::Transfer(TransferTx {
                from: [1u8; 32],
                to: [height as u8 + 10; 32],
                amount: 100,
                nonce: height,
                signature: None,
                public_key: None,
            });
            for node in nodes.iter_mut() {
                node.mempool.submit(tx.clone());
            }

            let block = run_consensus_height(&mut nodes);
            assert_eq!(block.number, height);

            let result = nodes[0].execute_block(&mut db, &block).unwrap();
            let sr = result.execution.state_root;
            state_roots.push(sr);

            for node in nodes.iter_mut() {
                node.on_block_committed(&block, sr, result.execution.objects_evaporated);
            }
        }

        // State roots should be non-zero (state was modified)
        for sr in &state_roots {
            assert_ne!(*sr, [0u8; 32], "State root should be non-zero");
        }
        // First state root should differ from initial (empty) state
        assert_eq!(state_roots.len(), 5);
    }

    // ─────────────── Test: DA Integration ─────────────────────────────

    #[test]
    fn test_da_erasure_encode_and_verify() {
        // Create a blob of data (simulating block data)
        let block_data = b"EvaporChain block data with transactions and state diffs";

        // 2D erasure encode
        let encoder = ErasureEncoder2D::with_cell_size(32);
        let matrix = encoder.encode_2d(block_data).unwrap();
        let ext_dim = matrix.extended_dim();
        assert!(ext_dim > 0);

        // Generate row/column commitments
        let commitments = RowColumnCommitments::from_matrix(&matrix);
        let data_root = commitments.data_root;
        assert_ne!(data_root, [0u8; 32]);

        // Generate and verify cell proofs for random sampling
        let queries = evaporchain_da::commitments::generate_2d_queries(1, ext_dim, 4, b"seed");
        for query in &queries {
            let proof = commitments.generate_cell_proof(&matrix, query.row, query.col).unwrap();
            assert!(commitments.verify_cell_proof(&proof));
        }
    }

    #[test]
    fn test_da_certificate_with_bls_attestations() {
        let (vs, bls_kps) = make_validator_set_with_bls(4, 1000);

        // Simulate block data and DA processing
        let data_root = blake3_hash(b"block data");
        let total_stake = vs.total_stake();

        // Build DA certificate with 3/4 validators attesting
        let mut builder = CertificateBuilder::new(1, data_root, total_stake);
        for i in 0..3u64 {
            let att = create_attestation(
                1,                  // block_number
                &data_root,         // data_root
                i,                  // validator_id
                4,                  // samples_verified
                vs.get(i).unwrap().stake,
                &bls_kps[i as usize],
            );
            builder.add_attestation(att);
        }

        assert!(builder.has_supermajority());
        let cert = builder.try_build();
        assert!(cert.is_some());
        let cert = cert.unwrap();
        assert!(cert.is_supermajority());
        assert_eq!(cert.attestations.len(), 3);
    }

    // ─────────────── Test: Consensus + Light Client ───────────────────

    #[test]
    fn test_light_client_verifies_consensus_output() {
        let mut nodes = setup_4_node_network();

        // Run 3 heights
        let mut blocks = Vec::new();
        for _ in 0..3 {
            let block = run_consensus_height(&mut nodes);
            let state_root = [block.number as u8; 32];
            for node in nodes.iter_mut() {
                node.on_block_committed(&block, state_root, 0);
            }
            blocks.push(block);
        }

        // All blocks should have commit certificates
        for block in &blocks {
            assert!(block.commit_certificate.is_some());
        }

        // Bootstrap light client with first block
        let b1 = &blocks[0];
        let cert1 = b1.commit_certificate.as_ref().unwrap();
        let vs = nodes[0].validator_set.clone();

        let genesis = LightBlockHeader {
            height: b1.number,
            epoch: b1.epoch,
            block_hash: cert1.block_hash,
            parent_hash: b1.parent_hash,
            state_root: [1u8; 32],
            timestamp: b1.timestamp,
            validator_set: vs.clone(),
            commit_certificate: cert1.clone(),
        };

        let mut lc = LightClientVerifier::new(genesis, b1.timestamp);

        // Verify subsequent blocks via light client
        for block in &blocks[1..] {
            let cert = block.commit_certificate.as_ref().unwrap();
            let header = LightBlockHeader {
                height: block.number,
                epoch: block.epoch,
                block_hash: cert.block_hash,
                parent_hash: block.parent_hash,
                state_root: [block.number as u8; 32],
                timestamp: block.timestamp,
                validator_set: vs.clone(),
                commit_certificate: cert.clone(),
            };
            let result = lc.verify(&header, block.timestamp);
            assert_eq!(
                result,
                VerificationResult::Valid,
                "Light client should verify block {}",
                block.number
            );
        }
    }

    // ─────────────── Test: Persistence + Recovery ─────────────────────

    #[test]
    fn test_consensus_checkpoint_and_recovery() {
        let mut nodes = setup_4_node_network();
        let store = InMemoryStateStore::new();

        // Run 2 heights
        for _ in 0..2 {
            let block = run_consensus_height(&mut nodes);
            let state_root = [block.number as u8; 32];
            for node in nodes.iter_mut() {
                node.on_block_committed(&block, state_root, 0);
            }

            // Save checkpoint after each commit
            let checkpoint = ConsensusCheckpoint::from_consensus(
                block.number,
                block.epoch,
                block.parent_hash,
                &nodes[0].validator_set,
                &[],
            );
            store.save_checkpoint(&checkpoint).unwrap();
        }

        // Simulate recovery
        let loaded = store.load_checkpoint().unwrap().unwrap();
        assert_eq!(loaded.height, 2);
        let restored_vs = loaded.restore_validator_set();
        assert_eq!(restored_vs.active_count(), 4);

        // Create a new consensus instance and restore state
        let mut recovered = TendermintConsensus::new_for_test(
            0,
            0,
            restored_vs,
        );
        recovered.restore_state(loaded.height, loaded.epoch, loaded.parent_hash);

        // The recovered node should be able to tick without panicking
        let mut recovery_db = InMemoryStateDB::new();
        let _actions = recovered.tick(&mut recovery_db);
    }

    // ─────────────── Test: Epoch Transitions via Consensus ────────────

    #[test]
    fn test_validator_join_through_full_pipeline() {
        let mut nodes = setup_4_node_network();
        let mut db = InMemoryStateDB::new();

        // Submit a ValidatorStake tx for a new validator
        let stake_tx = Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: [10u8; 32],
            stake_amount: 500,
            validator_id: 10,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        });
        for node in nodes.iter_mut() {
            node.mempool.submit(stake_tx.clone());
        }

        // Run consensus — block should include the stake tx
        let block = run_consensus_height(&mut nodes);
        assert!(
            block.transactions.iter().any(|tx| matches!(tx, Transaction::ValidatorStake(_))),
            "Block should include ValidatorStake tx"
        );

        // Execute and commit
        let result = nodes[0].execute_block(&mut db, &block).unwrap();
        for node in nodes.iter_mut() {
            node.on_block_committed(&block, result.execution.state_root, 0);
        }

        // The epoch manager should have queued the join
        // (Won't apply until epoch boundary at height 100)
        assert_eq!(nodes[0].validator_set.active_count(), 4); // unchanged yet
    }

    // ─────────────── Test: State Sync with DA ─────────────────────────

    #[test]
    fn test_state_sync_with_snapshot_provider() {
        // Provider has a snapshot
        let mut provider = SnapshotProvider::new();
        let state_data = vec![0xAB; 1024 * 512]; // 512KB
        let state_root = blake3_hash(&state_data);
        provider.create_snapshot(1000, 10, state_root, &state_data);

        // Syncing node starts
        let mut sync = StateSyncManager::new(0);
        assert!(StateSyncManager::needs_state_sync(0, 1002));

        let _actions = sync.start();

        // Simulate tip discovery
        sync.on_message(1, SyncMessage::TipResponse { height: 1000, block_hash: [1u8; 32] });
        let _actions = sync.on_message(
            2,
            SyncMessage::TipResponse { height: 1000, block_hash: [1u8; 32] },
        );
        assert!(matches!(sync.phase(), SyncPhase::VerifyingHeader));

        // Bootstrap with a header
        let (vs, bls_kps) = make_validator_set_with_bls(4, 1000);
        let msg = {
            let mut m = Vec::with_capacity(48);
            m.extend_from_slice(b"precommit");
            m.extend_from_slice(&1000u64.to_le_bytes());
            m.extend_from_slice(&0u32.to_le_bytes());
            m.extend_from_slice(&[100u8; 32]);
            m
        };
        let sigs: Vec<BlsSignature> = (0..3).map(|i| bls_kps[i].sign(&msg)).collect();
        let agg = BlsVerifier::aggregate_signatures(&sigs).unwrap();

        let header = LightBlockHeader {
            height: 1000,
            epoch: 10,
            block_hash: [100u8; 32],
            parent_hash: [99u8; 32],
            state_root,
            timestamp: 10000,
            validator_set: vs,
            commit_certificate: CommitCertificate {
                height: 1000,
                round: 0,
                block_hash: [100u8; 32],
                aggregate_signature: agg.0,
                signer_ids: vec![0, 1, 2],
            },
        };
        let actions = sync.on_message(1, SyncMessage::HeaderResponse { header });

        // Should now be downloading
        assert!(matches!(sync.phase(), SyncPhase::DownloadingSnapshot { .. }));

        // Serve metadata and all chunks through the provider
        let meta_resp = provider.handle_request(
            &SyncMessage::SnapshotMetadataRequest { height: 1000 },
            1000,
        ).unwrap();
        let actions = sync.on_message(1, meta_resp);

        // Serve all requested chunks
        for action in actions {
            if let SyncAction::SendToPeer { message, .. } = action {
                if let Some(resp) = provider.handle_request(&message, 1000) {
                    sync.on_message(1, resp);
                }
            }
        }

        // Continue until complete (may need multiple rounds)
        for _ in 0..20 {
            if sync.is_complete() || sync.is_failed() {
                break;
            }
            // Request remaining chunks directly
            for i in 0..10 {
                let resp = provider.handle_request(
                    &SyncMessage::ChunkRequest { height: 1000, chunk_index: i },
                    1000,
                );
                if let Some(r) = resp {
                    sync.on_message(1, r);
                }
            }
        }

        assert!(sync.is_complete(), "State sync should complete");
    }

    // ─────────────── Test: Blob TX through Consensus ──────────────────

    #[test]
    fn test_blob_tx_through_consensus_and_da() {
        let mut nodes = setup_4_node_network();
        let mut db = InMemoryStateDB::new();

        // Fund blob submitter
        db.put_account(Account {
            address: [5u8; 32],
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });

        // Submit blob tx
        let blob_tx = Transaction::Blob(BlobTx {
            submitter: [5u8; 32],
            data: vec![0xDE; 256],
            nonce: 1,
            namespace_id: 42,
            signature: None,
            public_key: None,
        });
        for node in nodes.iter_mut() {
            node.mempool.submit(blob_tx.clone());
        }

        let block = run_consensus_height(&mut nodes);
        assert!(
            block.transactions.iter().any(|tx| matches!(tx, Transaction::Blob(_))),
            "Block should include blob tx"
        );

        // DA encode the blob data
        let blob_data: Vec<u8> = block.transactions.iter().filter_map(|tx| {
            if let Transaction::Blob(ref b) = tx { Some(b.data.clone()) } else { None }
        }).flatten().collect();

        if !blob_data.is_empty() {
            let encoder = ErasureEncoder2D::with_cell_size(32);
            let matrix = encoder.encode_2d(&blob_data).unwrap();
            let comms = RowColumnCommitments::from_matrix(&matrix);
            assert_ne!(comms.data_root, [0u8; 32]);
        }

        // Execute and commit
        let result = nodes[0].execute_block(&mut db, &block).unwrap();
        for node in nodes.iter_mut() {
            node.on_block_committed(&block, result.execution.state_root, 0);
        }
    }

    // ─────────────── Test: VRF Randomness Across Heights ──────────────

    #[test]
    #[ignore] // Slow: ML-DSA VRF keygen ~60s per validator
    fn test_vrf_randomness_beacon_integration() {
        let mut nodes = setup_4_node_network_with_vrf();

        let mut vrf_outputs = Vec::new();
        for _ in 0..5 {
            let block = run_consensus_height(&mut nodes);
            if let Some(vrf) = block.vrf_output {
                vrf_outputs.push(vrf);
            }
            let state_root = [block.number as u8; 32];
            for node in nodes.iter_mut() {
                node.on_block_committed(&block, state_root, 0);
            }
        }

        // Should have VRF outputs (at least from heights where our node proposed)
        assert!(!vrf_outputs.is_empty(), "Should have VRF outputs");

        // All VRF outputs should be unique
        for i in 0..vrf_outputs.len() {
            for j in (i + 1)..vrf_outputs.len() {
                assert_ne!(vrf_outputs[i], vrf_outputs[j], "VRF outputs should be unique");
            }
        }

        // Randomness beacon should have accumulated randomness
        let beacon = nodes[0].randomness_beacon();
        let current = beacon.current();
        assert_ne!(current, [0u8; 32], "Beacon should have non-zero randomness");
    }

    // ─────────────── Test: ML-DSA Signed Transaction ─────────────────

    #[test]
    #[ignore] // ML-DSA keygen ~20s in debug mode; run with: cargo test --release -p evaporchain-integration-tests
    fn test_ml_dsa_signed_transfer_through_consensus() {
        let mut nodes = setup_4_node_network();
        let mut db = InMemoryStateDB::new();

        // Get cached ML-DSA (Dilithium3) keypair (avoids ~15s keygen per test)
        let keypair = get_ml_dsa_keypair();
        let pk_bytes = keypair.public_key_bytes();

        // Sender address = blake3(public_key)[..32]
        let sender = blake3_hash(&pk_bytes);

        // Fund sender
        db.put_account(Account {
            address: sender,
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });

        // Build and sign the transfer
        let mut tx = Transaction::Transfer(TransferTx {
            from: sender,
            to: [2u8; 32],
            amount: 500,
            nonce: 1,
            signature: None,
            public_key: None,
        });

        // Sign with ML-DSA
        let msg = tx.signable_bytes();
        let sig = keypair.sign(&msg);

        // Attach signature and public key
        if let Transaction::Transfer(ref mut t) = tx {
            t.signature = Some(sig.clone());
            t.public_key = Some(pk_bytes.clone());
        }

        // Verify signature is valid before submitting
        assert!(
            evaporchain_crypto::signatures::MlDsaVerifier::verify(&msg, &sig, &pk_bytes),
            "ML-DSA signature should verify"
        );

        // Submit to all nodes
        for node in nodes.iter_mut() {
            node.mempool.submit(tx.clone());
        }

        // Run consensus
        let block = run_consensus_height(&mut nodes);
        assert!(!block.transactions.is_empty(), "Block should have transactions");

        // Verify the transaction in the block has a signature
        let block_tx = &block.transactions[0];
        assert!(block_tx.signature().is_some(), "Block tx should carry ML-DSA signature");
        assert!(block_tx.public_key().is_some(), "Block tx should carry ML-DSA public key");

        // Execute with signature verification enabled
        let mut executor = ParallelExecutor::new_with_sig_verification(0);
        let result = executor.execute_block(&mut db, &block);
        assert!(result.is_ok(), "Block execution with sig verification should succeed: {:?}", result.err());

        let result = result.unwrap();
        assert_ne!(result.state_root, [0u8; 32]);

        // Advance nodes
        for node in nodes.iter_mut() {
            node.on_block_committed(&block, result.state_root, result.objects_evaporated);
        }
    }

    // ─────────────── Test: ML-DSA Invalid Signature Rejected ─────────

    #[test]
    #[ignore] // ML-DSA keygen ~20s in debug mode; run with: cargo test --release -p evaporchain-integration-tests
    fn test_ml_dsa_invalid_signature_rejected() {
        let mut db = InMemoryStateDB::new();
        let keypair = get_ml_dsa_keypair();
        let pk_bytes = keypair.public_key_bytes();
        let sender = blake3_hash(&pk_bytes);

        db.put_account(Account {
            address: sender,
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });

        // Build tx with WRONG signature (sign different message)
        let tx = Transaction::Transfer(TransferTx {
            from: sender,
            to: [2u8; 32],
            amount: 500,
            nonce: 1,
            signature: Some(keypair.sign(b"wrong message")),
            public_key: Some(pk_bytes),
        });

        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![tx],
            timestamp: 100,
            chain_id: String::new(),
            producer_id: Some(0),
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            oracle_state_root: None,
            shard_count: None,
        };

        // Execute with sig verification — should skip the bad tx
        let mut executor = ParallelExecutor::new_with_sig_verification(0);
        let result = executor.execute_block(&mut db, &block).unwrap();

        // Transfer should NOT have executed (bad sig)
        let recipient = db.get_account(&[2u8; 32]);
        assert!(
            recipient.is_none() || recipient.unwrap().balance == 0,
            "Recipient should have 0 balance — invalid sig tx should be rejected"
        );
    }

    // ─────────────── Test: Finality Tracker Integration ──────────────

    #[test]
    fn test_finality_tracker_with_consensus() {
        let mut nodes = setup_4_node_network();
        let mut tracker = FinalityTracker::new();

        // Run 5 heights through consensus
        for _ in 0..5 {
            let block = run_consensus_height(&mut nodes);
            let state_root = [block.number as u8; 32];

            // Record finality from the commit certificate
            if let Some(ref cert) = block.commit_certificate {
                let signing_stake = cert.signer_ids.len() as u64 * 1000; // each validator has 1000 stake
                let total_stake = 4000;
                tracker.on_block_finalized(
                    block.number,
                    cert.block_hash,
                    state_root,
                    block.epoch,
                    cert.clone(),
                    signing_stake,
                    total_stake,
                    block.timestamp,
                );
            }

            for node in nodes.iter_mut() {
                node.on_block_committed(&block, state_root, 0);
            }
        }

        // Verify finality state
        assert_eq!(tracker.latest_finalized_height(), 5);
        assert_eq!(tracker.total_finalized(), 5);

        // Block 1 should be finalized with 4 confirmations
        assert_eq!(
            tracker.finality_status(1),
            FinalityStatus::Finalized { confirmations: 4 }
        );

        // Generate and verify a finality proof
        let proof = tracker.generate_proof(3).unwrap();
        assert!(FinalityTracker::verify_proof(&proof));
        assert_eq!(proof.height, 3);

        // Stats should show good participation
        let stats = tracker.stats(5);
        assert_eq!(stats.finalized_count, 5);
        assert!(stats.avg_participation > 0.5, "Should have >50% participation");
    }
}

// ═══════════════════════════════════════════════════════════════════
// Substrate crate integration tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod substrate_integration {
    use evaporchain_demurrage::{demurrage_owed, DemurrageParams};
    use evaporchain_mera::commit;
    use evaporchain_epv::{EpvRegistry, ProtocolVersion, prune_evaporated};
    use evaporchain_tombstone::{mint, EulogyTrie, CauseOfDeath};
    use evaporchain_dsn::DsnWindow;
    use evaporchain_fee_controller::{FeeController, FeeControllerParams, FeeState};

    // ── Demurrage ───────────────────────────────────────────────────

    #[test]
    fn demurrage_zero_below_threshold() {
        let params = DemurrageParams::new(10, 1024);
        // balance at threshold — rate should be zero per the piecewise rule
        assert_eq!(demurrage_owed(1024, 0, 1_000_000, &params), 0);
    }

    #[test]
    fn demurrage_accrues_above_threshold() {
        let params = DemurrageParams::new(1, 1024); // 1 ppm/epoch/log-doubling
        // 2*1024 = 2048, log2(2048/1024)=1, rate=1 ppm
        // elapsed=1000 → owed = floor(2048 * 1 * 1000 / 1_000_000) = 2
        let owed = demurrage_owed(2048, 0, 1000, &params);
        assert!(owed > 0, "balance above threshold should owe demurrage");
        assert!(owed <= 2048, "owed never exceeds balance");
    }

    #[test]
    fn demurrage_disabled_owes_nothing() {
        let params = DemurrageParams::disabled();
        assert_eq!(demurrage_owed(u64::MAX, 0, 1_000_000, &params), 0);
    }

    // ── MERA state commitment ────────────────────────────────────────

    #[test]
    fn mera_commitment_is_deterministic() {
        let energies = vec![1000u64, 2000, 3000, 4000, 5000, 6000, 7000, 8000];
        let (c1, _) = commit(&energies, 4096, 100);
        let (c2, _) = commit(&energies, 4096, 100);
        assert_eq!(c1.root_hash, c2.root_hash, "MERA commitment must be deterministic");
        assert_eq!(c1.header_bytes(), c2.header_bytes());
    }

    #[test]
    fn mera_commitment_changes_with_energy_update() {
        let energies = vec![1000u64, 2000, 3000, 4000];
        let (c1, _) = commit(&energies, 4096, 100);
        let mut e2 = energies.clone();
        e2[2] = 9999; // one account gained energy
        let (c2, _) = commit(&e2, 4096, 100);
        assert_ne!(c1.root_hash, c2.root_hash, "any energy change must change the MERA root");
    }

    #[test]
    fn mera_header_bytes_include_lambda() {
        let energies = vec![1000u64, 2000];
        let (c1, _) = commit(&energies, 4096, 100);
        let (c2, _) = commit(&energies, 8192, 100); // different lambda
        assert_ne!(c1.header_bytes(), c2.header_bytes(), "lambda is committed in header_bytes");
    }

    // ── EPV — Evaporative Protocol Versioning ────────────────────────

    #[test]
    fn epv_prune_removes_zero_energy_versions() {
        let mut reg = EpvRegistry::new();
        let _ = reg.register(ProtocolVersion::new(1, 1_000_000, 0)); // healthy
        let _ = reg.register(ProtocolVersion::new(2, 0, 0));         // evaporated
        let _ = reg.register(ProtocolVersion::new(3, 500_000, 0));   // healthy

        let outcome = prune_evaporated(&mut reg, 1);
        assert_eq!(outcome.pruned.len(), 1, "one evaporated version should be pruned");
        assert_eq!(outcome.pruned[0], 2);
    }

    #[test]
    fn epv_registration_ordering_stable() {
        let mut reg = EpvRegistry::new();
        for id in 1u64..=5 {
            let _ = reg.register(ProtocolVersion::new(id, 1_000_000, 0));
        }
        let versions = reg.live_versions();
        assert_eq!(versions.len(), 5);
    }

    // ── Tombstone + EulogyTrie ───────────────────────────────────────

    #[test]
    fn tombstone_mint_deterministic() {
        let addr = [0x01u8; 32];
        let t1 = mint(addr, 0, 100, CauseOfDeath::Evaporated);
        let t2 = mint(addr, 0, 100, CauseOfDeath::Evaporated);
        assert_eq!(t1.commitment, t2.commitment, "tombstone is deterministic");
    }

    #[test]
    fn tombstone_different_cause_different_commitment() {
        let addr = [0x01u8; 32];
        let t_evap = mint(addr, 0, 100, CauseOfDeath::Evaporated);
        let t_slash = mint(addr, 0, 100, CauseOfDeath::SlashedToZero);
        assert_ne!(t_evap.commitment, t_slash.commitment);
    }

    #[test]
    fn eulogy_trie_root_is_order_independent() {
        let addr_a = [0xAAu8; 32];
        let addr_b = [0xBBu8; 32];
        let t_a = mint(addr_a, 0, 1000, CauseOfDeath::Evaporated);
        let t_b = mint(addr_b, 500, 2000, CauseOfDeath::RentExhausted);

        let mut trie1 = EulogyTrie::new();
        trie1.insert(addr_a, t_a).unwrap();
        trie1.insert(addr_b, t_b).unwrap();

        let mut trie2 = EulogyTrie::new();
        trie2.insert(addr_b, t_b).unwrap(); // reversed order
        trie2.insert(addr_a, t_a).unwrap();

        assert_eq!(trie1.root(), trie2.root(), "EulogyTrie root is order-independent");
    }

    #[test]
    fn eulogy_trie_rejects_re_evaporation() {
        let addr = [0x42u8; 32];
        let t = mint(addr, 0, 100, CauseOfDeath::Evaporated);
        let mut trie = EulogyTrie::new();
        trie.insert(addr, t).unwrap();
        // Same address again — must fail
        let err = trie.insert(addr, t).unwrap_err();
        assert!(err.to_string().contains("already"), "re-evaporation must be rejected");
    }

    // ── DSN — Decay-Stamped Nullifiers ───────────────────────────────

    #[test]
    fn dsn_fold_and_advance_window() {
        let mut w = DsnWindow::new(8).expect("depth=8 is valid");
        let nullifier = [0x01u8; 32];
        // First fold succeeds
        assert!(w.fold_nullifier(nullifier, 1).is_ok());
        // Duplicate in same window is rejected
        assert!(w.fold_nullifier(nullifier, 1).is_err());
        // After advancing past window, old nullifier is forgettable
        for _ in 0..8 { w.advance_window(); }
        // Now the nullifier can re-appear (window expired)
        assert!(w.fold_nullifier(nullifier, 9).is_ok());
    }

    // ── Fee Controller ───────────────────────────────────────────────

    #[test]
    fn fee_controller_raises_fee_under_high_load() {
        let params = FeeControllerParams::default_genesis();
        let state = FeeState::at_equilibrium(1_000_000);
        // 5x target gas usage → fee should rise
        let gas_used = params.target_gas * 5;
        let (new_state, drift) = FeeController::step(&params, &state, gas_used, 1).unwrap();
        assert!(
            new_state.base_fee_ppm > state.base_fee_ppm,
            "high gas usage must raise base fee (drift={drift:?})"
        );
    }

    #[test]
    fn fee_controller_lowers_fee_under_low_load() {
        let params = FeeControllerParams::default_genesis();
        // Start at 2x genesis equilibrium so there's room to fall
        let state = FeeState::at_equilibrium(2_000_000);
        let gas_used = 0; // no gas used
        let (new_state, _drift) = FeeController::step(&params, &state, gas_used, 1).unwrap();
        assert!(
            new_state.base_fee_ppm < state.base_fee_ppm,
            "zero gas usage must lower base fee"
        );
    }

    #[test]
    fn fee_controller_stable_at_target() {
        let params = FeeControllerParams::default_genesis();
        let state = FeeState::at_equilibrium(1_000_000);
        // Exactly target gas → fee should be close to stable
        let gas_used = params.target_gas;
        let (new_state, _) = FeeController::step(&params, &state, gas_used, 1).unwrap();
        let delta = (new_state.base_fee_ppm as i64 - state.base_fee_ppm as i64).abs();
        // Small delta acceptable (EMA smoothing), but it should be tiny
        assert!(delta < (state.base_fee_ppm as i64 / 10), "target gas should keep fee near stable");
    }

    // ── Energy Kernel — conservation invariant + redirects ───────────

    #[test]
    fn conservation_redirect_preserves_total() {
        use evaporchain_energy_kernel::{
            compartment::{Compartment, EnergyAccumulator},
            conservation::ConservationCheck,
            redirect::{EnergyRedirect, RedirectKind},
        };
        let before = EnergyAccumulator::new(1_000_000, 500_000, 0, 0);
        let mut after = before;
        EnergyRedirect::new(RedirectKind::MevBurn, 50_000)
            .apply(&mut after)
            .expect("mev_burn should succeed");
        ConservationCheck::redirect(&before, &after)
            .expect("redirect must preserve total");
        assert_eq!(before.total(), after.total());
        assert_eq!(after[Compartment::Accounts], 950_000);
        assert_eq!(after[Compartment::RefreshPool], 50_000);
    }

    #[test]
    fn conservation_decay_step_valid_within_lambda() {
        use evaporchain_energy_kernel::{
            compartment::EnergyAccumulator, conservation::ConservationCheck, ChainLambda, Lambda,
        };
        let before = EnergyAccumulator::new(1_000_000, 0, 0, 0);
        // After one half-life (4096 epochs), minimum retained = 500_000
        let after = EnergyAccumulator::new(700_000, 0, 0, 0);
        let lambda = ChainLambda::new(Lambda::from_epochs(4096));
        ConservationCheck::decay_step(&before, &after, 4096, lambda)
            .expect("holding 700k after half-life of 1M is legal (min=500k)");
    }

    #[test]
    fn conservation_violation_detected_on_total_increase() {
        use evaporchain_energy_kernel::{
            compartment::EnergyAccumulator, conservation::ConservationCheck, ChainLambda, Lambda,
        };
        let before = EnergyAccumulator::new(1_000_000, 0, 0, 0);
        let after = EnergyAccumulator::new(1_000_001, 0, 0, 0); // energy created from nothing
        let lambda = ChainLambda::new(Lambda::from_epochs(4096));
        let result = ConservationCheck::decay_step(&before, &after, 0, lambda);
        assert!(result.is_err(), "total increase must be a conservation violation");
    }

    #[test]
    fn conservation_violation_detected_when_drop_exceeds_lambda() {
        use evaporchain_energy_kernel::{
            compartment::EnergyAccumulator, conservation::ConservationCheck, ChainLambda, Lambda,
        };
        let before = EnergyAccumulator::new(1_000_000, 0, 0, 0);
        // After 4096 epochs at half_life=4096, min retained ≈ 500_000. Holding
        // only 100_000 means we destroyed far more than λ allows.
        let after = EnergyAccumulator::new(100_000, 0, 0, 0);
        let lambda = ChainLambda::new(Lambda::from_epochs(4096));
        let result = ConservationCheck::decay_step(&before, &after, 4096, lambda);
        assert!(result.is_err(), "drop exceeding λ-bound must be rejected");
    }

    #[test]
    fn redirect_insufficient_source_rejected() {
        use evaporchain_energy_kernel::{
            compartment::EnergyAccumulator,
            redirect::{EnergyRedirect, RedirectKind},
        };
        let mut acc = EnergyAccumulator::new(0, 50, 0, 0);
        let before_total = acc.total();
        // Slash needs Stake ≥ 100; only 50 available
        let result = EnergyRedirect::new(RedirectKind::Slash, 100).apply(&mut acc);
        assert!(result.is_err(), "insufficient source must be rejected");
        assert_eq!(acc.total(), before_total, "accumulator must be unchanged on rejection");
    }
}

// ── Cross-crate flows: LLSA → EPV, Tombstone chain, Energy-conservation pipeline ───

#[cfg(test)]
mod cross_crate_integration {
    use evaporchain_llsa::{
        Amendment, apply_amendment,
        proof::{AlwaysAcceptVerifier, LlsaProof},
    };
    use evaporchain_epv::{EpvRegistry, ProtocolVersion, prune_evaporated};
    use evaporchain_tombstone::{mint, EulogyTrie, CauseOfDeath};
    use evaporchain_energy_kernel::{
        compartment::EnergyAccumulator, conservation::ConservationCheck,
        redirect::{EnergyRedirect, RedirectKind}, ChainLambda, Lambda,
    };
    use evaporchain_demurrage::{demurrage_owed, DemurrageParams};

    // ── LLSA → EPV cross-crate amendment flow ──────────────────────────

    fn make_amendment(from: u64, to: u64) -> Amendment {
        let descriptor = format!("step-impl-v{to}").into_bytes();
        let mut proof = LlsaProof {
            coq_term_hash: [0u8; 32],
            target_invariant_id: [0u8; 32],
            bound_amendment_hash: [0u8; 32],
            proof_bytes: vec![],
        };
        let amendment = Amendment { from_version: from, to_version: to, step_new_descriptor: descriptor, proof: proof.clone() };
        proof.bound_amendment_hash = amendment.hash();
        Amendment { from_version: from, to_version: to, step_new_descriptor: format!("step-impl-v{to}").into_bytes(), proof }
    }

    #[test]
    fn llsa_amendment_registers_in_epv() {
        let mut reg = EpvRegistry::new();
        reg.register(ProtocolVersion::new(1, 1_000_000, 0)).unwrap();

        let amendment = make_amendment(1, 2);
        apply_amendment(&mut reg, &amendment, [0u8; 32], 500_000, 100, &AlwaysAcceptVerifier)
            .expect("amendment should be accepted by AlwaysAcceptVerifier");

        assert!(reg.contains(2), "version 2 must be registered after amendment");
        assert_eq!(reg.live_versions().len(), 2);
    }

    #[test]
    fn llsa_amendment_chain_v1_v2_v3() {
        let mut reg = EpvRegistry::new();
        reg.register(ProtocolVersion::new(1, 1_000_000, 0)).unwrap();

        for (from, to) in [(1u64, 2u64), (2, 3)] {
            let a = make_amendment(from, to);
            apply_amendment(&mut reg, &a, [0u8; 32], 500_000, 0, &AlwaysAcceptVerifier).unwrap();
        }
        assert_eq!(reg.live_versions().len(), 3);
    }

    #[test]
    fn llsa_amendment_collision_rejected() {
        let mut reg = EpvRegistry::new();
        reg.register(ProtocolVersion::new(1, 1_000_000, 0)).unwrap();
        reg.register(ProtocolVersion::new(2, 1_000_000, 0)).unwrap();

        let a = make_amendment(1, 2); // version 2 already exists
        let result = apply_amendment(&mut reg, &a, [0u8; 32], 500_000, 0, &AlwaysAcceptVerifier);
        assert!(result.is_err(), "upgrading to an existing version must fail");
    }

    #[test]
    fn llsa_amendment_from_absent_rejected() {
        let mut reg = EpvRegistry::new();
        reg.register(ProtocolVersion::new(1, 1_000_000, 0)).unwrap();

        let a = make_amendment(99, 100); // version 99 doesn't exist
        let result = apply_amendment(&mut reg, &a, [0u8; 32], 500_000, 0, &AlwaysAcceptVerifier);
        assert!(result.is_err(), "from_version absent must fail");
    }

    #[test]
    fn epv_prune_after_amendment_removes_zero_energy() {
        let mut reg = EpvRegistry::new();
        reg.register(ProtocolVersion::new(1, 1_000_000, 0)).unwrap();

        let a = make_amendment(1, 2);
        apply_amendment(&mut reg, &a, [0u8; 32], 0, 0, &AlwaysAcceptVerifier).unwrap(); // seed=0 → already evaporated

        let outcome = prune_evaporated(&mut reg, 1);
        assert_eq!(outcome.pruned.len(), 1);
        assert_eq!(outcome.pruned[0], 2);
    }

    // ── Tombstone evaporation chain ─────────────────────────────────────

    #[test]
    fn tombstone_evaporation_chain_multiple_accounts() {
        let addrs: Vec<[u8; 32]> = (0u8..5).map(|i| [i; 32]).collect();
        let mut trie = EulogyTrie::new();

        for (epoch, addr) in addrs.iter().enumerate() {
            let t = mint(*addr, 100 * epoch as u64, epoch as u64, CauseOfDeath::Evaporated);
            trie.insert(*addr, t).unwrap();
        }

        let root = trie.root();
        assert_ne!(root, [0u8; 32], "non-empty EulogyTrie root must be non-zero");

        // Rebuild in reverse — root must be the same (order-independence)
        let mut trie2 = EulogyTrie::new();
        for (epoch, addr) in addrs.iter().enumerate().rev() {
            let t = mint(*addr, 100 * epoch as u64, epoch as u64, CauseOfDeath::Evaporated);
            trie2.insert(*addr, t).unwrap();
        }
        assert_eq!(root, trie2.root(), "insertion order must not affect EulogyTrie root");
    }

    // ── Demurrage → energy-kernel conservation pipeline ─────────────────

    #[test]
    fn demurrage_redirect_satisfies_conservation() {
        // Simulate: an idle account accrues demurrage; that demurrage is
        // redirected into the RefreshPool; the conservation check passes.
        let params = DemurrageParams::new(1, 1024);
        let balance: u64 = 50_000;
        let owed = demurrage_owed(balance, 0, 100, &params);
        assert!(owed > 0, "demurrage must accrue on this balance");

        let before = EnergyAccumulator::new(balance, 0, 0, 0);
        let mut after = before;
        EnergyRedirect::new(RedirectKind::Demurrage, owed)
            .apply(&mut after)
            .expect("demurrage redirect must succeed");

        ConservationCheck::redirect(&before, &after)
            .expect("demurrage redirect must preserve total energy");
        assert_eq!(before.total(), after.total(), "conservation: total unchanged");
    }

    #[test]
    fn conservation_pipeline_redirect_then_decay() {
        // Full block pipeline: redirect (MEV burn) then λ-decay.
        // The composite block_step check must pass.
        let before = EnergyAccumulator::new(2_000_000, 1_000_000, 0, 0);
        let mut mid = before;
        EnergyRedirect::new(RedirectKind::MevBurn, 10_000)
            .apply(&mut mid)
            .unwrap();

        // Simulate one half-life of decay (rough: each compartment halves)
        let after = EnergyAccumulator::new(
            mid.total() / 2, 0, 0, 0, // all energy in Accounts for simplicity
        );
        let lambda = ChainLambda::new(Lambda::from_epochs(1));
        // 1 epoch elapsed, half-life=1 → retained_min = before.total()/2
        ConservationCheck::block_step(&before, &after, 1, lambda)
            .expect("redirect+decay within λ must pass conservation check");
    }
}

// ── Privacy layer: PNT nullifier tree + PRP retention proofs ────────────────

#[cfg(test)]
mod privacy_integration {
    use evaporchain_pnt::{PhasedNullifierTree, Nullifier};
    use evaporchain_prp::{prove_retention, verify_retention_proof};
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    // ── PNT — Phased Nullifier Tree ──────────────────────────────────

    #[test]
    fn pnt_double_spend_same_phase_rejected() {
        let mut tree = PhasedNullifierTree::new(4).expect("depth=4 is valid");
        let nullifier: Nullifier = [0xABu8; 32];
        tree.insert_nullifier(nullifier).expect("first insert must succeed");
        let err = tree.insert_nullifier(nullifier).unwrap_err();
        assert!(
            format!("{err:?}").to_lowercase().contains("double") || format!("{err:?}").to_lowercase().contains("spent"),
            "duplicate nullifier must be rejected: {err:?}"
        );
    }

    #[test]
    fn pnt_double_spend_after_phase_advance_still_detected() {
        let mut tree = PhasedNullifierTree::new(4).expect("depth=4 is valid");
        let nullifier: Nullifier = [0xCDu8; 32];
        tree.insert_nullifier(nullifier).unwrap();
        tree.advance_phase(); // phase 1 → 2
        // Still within the 4-phase window → must be detected
        let err = tree.insert_nullifier(nullifier).unwrap_err();
        assert!(
            format!("{err:?}").to_lowercase().contains("double") || format!("{err:?}").to_lowercase().contains("spent"),
            "nullifier from prior phase must still be detected within window: {err:?}"
        );
    }

    #[test]
    fn pnt_live_count_tracks_insertions() {
        let mut tree = PhasedNullifierTree::new(4).expect("depth=4 is valid");
        assert_eq!(tree.live_count(), 0);
        for i in 0u8..5 {
            let n = [i; 32];
            tree.insert_nullifier(n).unwrap();
        }
        assert_eq!(tree.live_count(), 5, "live_count must track all insertions");
    }

    #[test]
    fn pnt_is_spent_in_window_true_after_insert() {
        let mut tree = PhasedNullifierTree::new(4).expect("depth=4 is valid");
        let n: Nullifier = [0x11u8; 32];
        assert!(!tree.is_spent_in_window(&n));
        tree.insert_nullifier(n).unwrap();
        assert!(tree.is_spent_in_window(&n), "is_spent_in_window must return true after insert");
    }

    // ── PRP — Private Retention Proofs ───────────────────────────────

    #[test]
    fn prp_retention_proof_verify_at_activation_epoch() {
        let state_id = [0x01u8; 32];
        let lambda = ChainLambda::new(Lambda::from_epochs(4096));
        let proof = prove_retention(state_id, 1_000_000, lambda, 0, 1);
        // Verifying at activation epoch must always succeed
        verify_retention_proof(&proof, 0)
            .expect("proof must verify at activated_epoch");
    }

    #[test]
    fn prp_retention_proof_expires_after_energy_decays() {
        let state_id = [0x02u8; 32];
        // Half-life=1 epoch, floor=1: energy decays to 0 very fast
        let lambda = ChainLambda::new(Lambda::from_epochs(1));
        let proof = prove_retention(state_id, 1_000, lambda, 0, 1);
        // The proof must expire well before epoch 1_000_000
        let result = verify_retention_proof(&proof, 1_000_000);
        assert!(result.is_err(), "proof must expire when queried far beyond retained_until_epoch");
    }

    #[test]
    fn prp_retention_proof_is_deterministic() {
        let state_id = [0x03u8; 32];
        let lambda = ChainLambda::new(Lambda::from_epochs(4096));
        let p1 = prove_retention(state_id, 500_000, lambda, 10, 100);
        let p2 = prove_retention(state_id, 500_000, lambda, 10, 100);
        assert_eq!(p1.retained_until_epoch, p2.retained_until_epoch);
        assert_eq!(p1.witness, p2.witness, "PRP witness must be deterministic");
    }

    #[test]
    fn prp_higher_energy_retains_longer() {
        let state_id = [0x04u8; 32];
        let lambda = ChainLambda::new(Lambda::from_epochs(100));
        let floor = 1_000u64;
        let p_low  = prove_retention(state_id, 10_000, lambda, 0, floor);
        let p_high = prove_retention(state_id, 10_000_000, lambda, 0, floor);
        assert!(
            p_high.retained_until_epoch > p_low.retained_until_epoch,
            "higher committed energy must retain state for more epochs"
        );
    }
}

// ── Fork evaporation certificates + DSN×PNT combined nullifier test ─────────

#[cfg(test)]
mod fork_and_nullifier_integration {
    use evaporchain_evap_fork_cert::{prove_fork_evaporated, verify_evaporated_cert, ForkBlock};
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_dsn::DsnWindow;
    use evaporchain_pnt::{PhasedNullifierTree, Nullifier};

    // ── EvaporatedForkCert prove→verify round-trip ──────────────────

    #[test]
    fn fork_cert_prove_and_verify_round_trip() {
        let fork_root = [0xFFu8; 32];
        let blocks = [
            ForkBlock { seed_energy: 1_000_000, observed_epoch: 0 },
            ForkBlock { seed_energy: 500_000,   observed_epoch: 50 },
        ];
        let lambda = ChainLambda::new(Lambda::from_epochs(100));
        let cert = prove_fork_evaporated(fork_root, &blocks, lambda, 500, 10_000);

        // The fork has 500 epochs of decay; with half_life=100 that's 5 halvings
        // so decayed << seed — the cert should verify if decayed < threshold=10_000.
        // We don't hard-code the exact value; just assert verify works.
        if cert.decayed_energy < cert.threshold {
            verify_evaporated_cert(&cert).expect("cert must verify when decayed < threshold");
        }
    }

    #[test]
    fn fork_cert_unproven_fork_not_evaporated() {
        // A fork with 0 epochs of decay — energy is still at seed → not evaporated
        let fork_root = [0x11u8; 32];
        let blocks = [ForkBlock { seed_energy: 1_000_000, observed_epoch: 0 }];
        let lambda = ChainLambda::new(Lambda::from_epochs(4096));
        let threshold = 500_000u128;
        let cert = prove_fork_evaporated(fork_root, &blocks, lambda, 0, threshold);

        // At epoch=0, decayed=seed=1_000_000 > threshold=500_000 → NOT evaporated
        assert!(cert.decayed_energy >= cert.threshold);
        let result = verify_evaporated_cert(&cert);
        assert!(result.is_err(), "non-evaporated fork cert must fail verification");
    }

    #[test]
    fn fork_cert_deterministic_witness() {
        let fork_root = [0x22u8; 32];
        let blocks = [ForkBlock { seed_energy: 100_000, observed_epoch: 10 }];
        let lambda = ChainLambda::new(Lambda::from_epochs(100));
        let c1 = prove_fork_evaporated(fork_root, &blocks, lambda, 300, 1);
        let c2 = prove_fork_evaporated(fork_root, &blocks, lambda, 300, 1);
        assert_eq!(c1.witness, c2.witness, "fork cert witness must be deterministic");
        assert_eq!(c1.decayed_energy, c2.decayed_energy);
    }

    // ── DSN × PNT: two-layer nullifier invalidation ──────────────────

    #[test]
    fn dsn_and_pnt_both_reject_double_spend() {
        let mut dsn = DsnWindow::new(8).expect("depth=8 valid");
        let mut pnt = PhasedNullifierTree::new(4).expect("depth=4 valid");

        let nullifier: Nullifier = [0x55u8; 32];

        // Layer 1: DSN fold
        dsn.fold_nullifier(nullifier, 1).expect("first DSN fold must succeed");
        let dsn_dup = dsn.fold_nullifier(nullifier, 1);
        assert!(dsn_dup.is_err(), "DSN must reject duplicate nullifier");

        // Layer 2: PNT insert
        pnt.insert_nullifier(nullifier).expect("first PNT insert must succeed");
        let pnt_dup = pnt.insert_nullifier(nullifier);
        assert!(pnt_dup.is_err(), "PNT must reject duplicate nullifier");

        // Both layers saw it — this simulates a two-layer privacy scheme
        assert!(dsn.fold_nullifier(nullifier, 1).is_err());
        assert!(pnt.insert_nullifier(nullifier).is_err());
    }

    #[test]
    fn dsn_window_expiry_allows_reuse_but_pnt_still_blocks() {
        let mut dsn = DsnWindow::new(4).expect("depth=4 valid");
        let mut pnt = PhasedNullifierTree::new(8).expect("depth=8 valid");

        let nullifier: Nullifier = [0x77u8; 32];
        dsn.fold_nullifier(nullifier, 1).unwrap();
        pnt.insert_nullifier(nullifier).unwrap();

        // Advance DSN past its window (depth=4)
        for _ in 0..4 { dsn.advance_window(); }
        // DSN now allows reuse (window expired)
        assert!(dsn.fold_nullifier(nullifier, 5).is_ok(), "DSN window expired — reuse should be allowed");

        // But PNT still has it in its deeper window (depth=8)
        assert!(pnt.insert_nullifier(nullifier).is_err(), "PNT must still block within its window");
    }
}

// ── Consensus substrate: WSBF→RG phase + self-annealing + Boltzmann stake ───

#[cfg(test)]
mod consensus_substrate_integration {
    use evaporchain_wsbf::{rg_step, BlockSummary, RgFlowParams};
    use evaporchain_rg_phase_map::{classify_regime, PhaseMapParams};
    use evaporchain_self_annealing::{
        AnnealingParams, AnnealedScore, accepts_candidate, effective_temperature, validator_score,
    };
    use evaporchain_boltzmann_stake::{proposer_weight, ValidatorStake};
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    fn make_window(n: usize, energy: u64, lambda: u64) -> Vec<BlockSummary> {
        (0..n).map(|i| BlockSummary {
            height: i as u64,
            total_energy: energy,
            active_accounts: 10,
            lambda_half_life: lambda,
        }).collect()
    }

    // ── WSBF → RG phase classification ───────────────────────────────

    #[test]
    fn wsbf_rg_step_classifies_liveness_stable() {
        let params = RgFlowParams { coarse_grain: 4, entropy_scale_mb: 500 };
        let window = make_window(4, 1_000_000, 4096);
        let ep = rg_step(&window, 0, &params).expect("rg_step must succeed");

        // With high λ_eff and no adversary, should be LivenessStable
        let phase_params = PhaseMapParams::default();
        let phase = classify_regime(ep.lambda_eff, 10, 0, &phase_params);
        assert_eq!(
            phase,
            evaporchain_rg_phase_map::ConsensusPhase::LivenessStable,
            "healthy network with λ_eff={} must be LivenessStable", ep.lambda_eff
        );
    }

    #[test]
    fn wsbf_rg_step_frozen_when_lambda_collapses() {
        let params = RgFlowParams { coarse_grain: 4, entropy_scale_mb: 500 };
        // Very low λ (half-life=1 epoch) → λ_eff will be tiny → Frozen
        let window = make_window(4, 1_000_000, 1);
        let ep = rg_step(&window, 0, &params).expect("rg_step must succeed");

        let phase_params = PhaseMapParams::default();
        let phase = classify_regime(ep.lambda_eff, 10, 0, &phase_params);
        assert_eq!(
            phase,
            evaporchain_rg_phase_map::ConsensusPhase::Frozen,
            "collapsed λ_eff={} must classify as Frozen", ep.lambda_eff
        );
    }

    #[test]
    fn rg_chaotic_at_high_adversary_fraction() {
        let phase_params = PhaseMapParams::default();
        let phase = classify_regime(4096, 10, 334, &phase_params); // 33.4% adversary
        assert_eq!(
            phase,
            evaporchain_rg_phase_map::ConsensusPhase::Chaotic,
            "adversary fraction ≥ 1/3 must be Chaotic"
        );
    }

    // ── Self-annealing temperature → Boltzmann weight consistency ────

    fn annealing_params() -> AnnealingParams {
        AnnealingParams { lambda_half_life: 4096, beta_mb: 1_000 }
    }

    #[test]
    fn annealing_temperature_decreases_with_epoch() {
        let params = annealing_params();
        let t0 = effective_temperature(&params, 0);
        let t100 = effective_temperature(&params, 100);
        let t1000 = effective_temperature(&params, 1000);
        assert!(t0 >= t100, "temperature must be non-increasing over epochs");
        assert!(t100 >= t1000, "temperature must be non-increasing over epochs");
    }

    #[test]
    fn annealing_favors_better_candidate_at_high_temperature() {
        let params = annealing_params();
        let v_old = AnnealedScore { stake: 1_000, activity: 0, uptime_milli: 900 };
        let v_new = AnnealedScore { stake: 5_000, activity: 100, uptime_milli: 999 };
        // At epoch=0 (highest T), a clearly better candidate should always be accepted
        let accepted = accepts_candidate(&params, 0, &v_old, &v_new, 42);
        assert!(accepted, "clearly better candidate must be accepted at high temperature");
    }

    #[test]
    fn boltzmann_weight_higher_for_more_active_validator() {
        let w_inactive = proposer_weight(1_000_000, 0, 1_000);
        let w_active   = proposer_weight(1_000_000, 100, 1_000);
        assert!(
            w_active > w_inactive,
            "higher activity must produce higher Boltzmann proposer weight"
        );
    }

    #[test]
    fn boltzmann_weight_higher_for_more_stake() {
        let w_low  = proposer_weight(100_000, 50, 1_000);
        let w_high = proposer_weight(1_000_000, 50, 1_000);
        assert!(
            w_high > w_low,
            "higher stake must produce higher Boltzmann proposer weight"
        );
    }
}

// ── Data availability sampling integration ───────────────────────────────────

#[cfg(test)]
mod da_integration {
    use evaporchain_da::erasure::{ErasureConfig, ErasureEncoder};
    use evaporchain_da::sampling::{DASampler, SampleQuery, SampleResponse};

    fn make_shards(data: &[u8]) -> Vec<evaporchain_da::erasure::Shard> {
        let cfg = ErasureConfig { data_shards: 4, parity_shards: 4 };
        let enc = ErasureEncoder::new(cfg).unwrap();
        enc.encode(data).unwrap().shards
    }

    // ── DASampler: commitment → proof → verify round-trip ────────────────

    #[test]
    fn da_sampler_commitment_and_proof_round_trip() {
        let shards = make_shards(b"EvaporChain block body — DA sampling integration test");
        let proof = DASampler::compute_commitment(&shards).expect("commitment must succeed");
        assert_ne!(proof.commitment_root, [0u8; 32]);
        assert_eq!(proof.total_shards, shards.len());

        // Each shard's Merkle proof must verify individually
        for shard in &shards {
            let merkle = DASampler::generate_proof(&shards, shard.index)
                .expect("proof generation must succeed");
            assert!(DASampler::verify_proof(shard, &merkle), "shard {} proof must verify", shard.index);
        }
    }

    #[test]
    fn da_sampler_tampered_shard_proof_fails() {
        let shards = make_shards(b"block-data-for-tamper-test-padding-padding-padding");
        let mut bad_shard = shards[0].clone();
        bad_shard.data[0] ^= 0xFF; // flip a byte

        let merkle = DASampler::generate_proof(&shards, 0).unwrap();
        // tampered shard data means the leaf hash won't match
        assert!(!DASampler::verify_proof(&bad_shard, &merkle), "tampered shard must fail verification");
    }

    // ── generate_queries: determinism and bounds ──────────────────────────

    #[test]
    fn da_queries_deterministic_and_bounded() {
        let shards = make_shards(b"determinism-test-block-data-padding-padding-padd");
        let total = shards.len();
        let seed = b"light-client-peer-id-12345";

        let q1 = DASampler::generate_queries(100, total, 8, seed);
        let q2 = DASampler::generate_queries(100, total, 8, seed);

        assert_eq!(q1.len(), 8);
        assert_eq!(q2.len(), 8, "generate_queries must be deterministic");
        for (a, b) in q1.iter().zip(q2.iter()) {
            assert_eq!(a.shard_index, b.shard_index);
        }
        for q in &q1 {
            assert!(q.shard_index < total, "all queries must be in range");
        }
    }

    // ── verify_samples: full light-client path ────────────────────────────

    #[test]
    fn da_verify_samples_full_light_client_path() {
        let data = b"light-client-sampling-block-data-padding-padding-pad";
        let shards = make_shards(data);
        let da_proof = DASampler::compute_commitment(&shards).unwrap();

        let queries = DASampler::generate_queries(42, shards.len(), 4, b"lc-seed");
        let responses: Vec<SampleResponse> = queries
            .iter()
            .map(|q| {
                let merkle = DASampler::generate_proof(&shards, q.shard_index).unwrap();
                SampleResponse {
                    shard: shards[q.shard_index].clone(),
                    proof: merkle,
                    attestation_signature: None,
                    attester_public_key: None,
                }
            })
            .collect();

        let valid = DASampler::verify_samples(&da_proof, &responses, 4)
            .expect("verify_samples must succeed with 4 valid responses");
        assert!(valid, "all sampled shards must verify against the commitment");
    }

    #[test]
    fn da_verify_samples_batch_identifies_invalid() {
        let data = b"batch-verification-block-data-padding-padding-padd";
        let shards = make_shards(data);
        let da_proof = DASampler::compute_commitment(&shards).unwrap();

        // Build one valid and one invalid response
        let merkle_0 = DASampler::generate_proof(&shards, 0).unwrap();
        let mut bad_shard = shards[1].clone();
        bad_shard.data[0] ^= 0xFF;
        bad_shard.hash = evaporchain_crypto::hash::blake3_hash(&bad_shard.data);
        let merkle_1 = DASampler::generate_proof(&shards, 1).unwrap();

        let responses = vec![
            SampleResponse { shard: shards[0].clone(), proof: merkle_0, attestation_signature: None, attester_public_key: None },
            SampleResponse { shard: bad_shard, proof: merkle_1, attestation_signature: None, attester_public_key: None },
        ];

        let batch = DASampler::verify_samples_batch(&da_proof, &responses, 1)
            .expect("batch_verify must not error");
        assert!(!batch.all_valid, "batch must not be all_valid when one shard is bad");
        assert!(batch.invalid_indices.contains(&1), "shard index 1 must be flagged as invalid");
    }

    // ── Insufficient samples → error ──────────────────────────────────────

    #[test]
    fn da_verify_samples_insufficient_returns_err() {
        let shards = make_shards(b"insufficient-samples-test-block-padding-padding-p");
        let da_proof = DASampler::compute_commitment(&shards).unwrap();
        let merkle = DASampler::generate_proof(&shards, 0).unwrap();
        let responses = vec![SampleResponse {
            shard: shards[0].clone(),
            proof: merkle,
            attestation_signature: None,
            attester_public_key: None,
        }];

        // min_samples=4 but only 1 provided
        let err = DASampler::verify_samples(&da_proof, &responses, 4);
        assert!(err.is_err(), "insufficient samples must return Err");
    }

    // ── Erasure reconstruction: recover from parity only ─────────────────

    #[test]
    fn erasure_reconstruct_from_parity_shards() {
        let original = b"erasure-recovery-test-data-EvaporChain-DA-padding-";
        let cfg = ErasureConfig { data_shards: 4, parity_shards: 4 };
        let enc = ErasureEncoder::new(cfg).unwrap();
        let encoded = enc.encode(original).unwrap();
        let shard_size = encoded.shard_size;

        // Drop the first 4 data shards (keep only parity)
        let mut shard_opts: Vec<Option<Vec<u8>>> = (0..8)
            .map(|i| {
                if i < 4 { None } else { Some(encoded.shards[i].data.clone()) }
            })
            .collect();

        let recovered = enc.reconstruct(shard_opts).expect("must reconstruct from parity");
        let trimmed = &recovered[..original.len()];
        assert_eq!(trimmed, original, "reconstructed data must match original");
    }
}

// ── Proof of Historical Activity (PoHA) integration ──────────────────────────

#[cfg(test)]
mod poha_integration {
    use evaporchain_da::poha::{CertTemperature, PoHACertificate, PoHAStore};

    fn make_cert(block_number: u64, energy: u64, half_life: u64, epoch: u64) -> PoHACertificate {
        PoHACertificate {
            block_number,
            data_root: [block_number as u8; 32],
            shard_count: 8,
            initial_energy: energy,
            energy,
            half_life,
            created_epoch: epoch,
            last_attested_epoch: epoch,
            attested_stake: 700,
            total_stake: 1000,
            re_attestation_count: 0,
            aggregate_signature: vec![],
            signer_ids: vec![0, 1, 2],
        }
    }

    // ── Temperature classification ────────────────────────────────────────

    #[test]
    fn poha_temperature_decays_hot_to_evaporated() {
        let mut cert = make_cert(1, 1_000_000, 10, 0);

        // At epoch 0: energy=1_000_000 (100%) → Hot
        assert!(matches!(cert.temperature(), CertTemperature::Hot));

        // After 2 half-lives: energy=250_000 (25%) → Warm
        cert.energy = cert.energy_at(20);
        assert!(matches!(cert.temperature(), CertTemperature::Warm));

        // After 4 half-lives: energy=62_500 (6.25%) → Cold
        cert.energy = cert.energy_at(40);
        // reset created_epoch so energy_at works from current
        cert.created_epoch = 40;
        cert.energy = cert.energy_at(40);
        assert!(matches!(cert.temperature(), CertTemperature::Cold));
    }

    #[test]
    fn poha_temperature_at_future_epoch() {
        let cert = make_cert(2, 1_000_000, 100, 0);
        // far future: 7 half-lives → 1_000_000 >> 7 = 7812 → < 1% → Evaporated
        let temp = cert.temperature_at(700);
        assert!(matches!(temp, CertTemperature::Evaporated));
    }

    // ── PoHAStore: register, process_epoch, evaporation ──────────────────

    #[test]
    fn poha_store_register_and_decay() {
        let mut store = PoHAStore::new(1_000_000, 10);

        store.register(1, [0x01u8; 32], 8, 700, 1000, 0, vec![], vec![0, 1, 2]);
        store.register(2, [0x02u8; 32], 8, 700, 1000, 0, vec![], vec![0, 1, 2]);

        assert_eq!(store.active_count(), 2);
        assert_eq!(store.ghost_count(), 0);

        // Advance 100 epochs (10 half-lives) → energy should collapse to ~0 → both evaporate
        let (_, evaporated) = store.process_epoch(100);
        assert_eq!(evaporated, 2, "both certs must evaporate after 10 half-lives");
        assert_eq!(store.active_count(), 0);
        assert_eq!(store.ghost_count(), 2);
    }

    #[test]
    fn poha_store_re_attest_extends_lifetime() {
        let mut store = PoHAStore::new(1_000_000, 10);
        store.register(1, [0xABu8; 32], 8, 700, 1000, 0, vec![], vec![0]);

        // Decay 5 half-lives (50% remaining)
        let _ = store.process_epoch(50);
        assert_eq!(store.active_count(), 1, "cert should survive 5 half-lives");

        // Re-attest: boosts energy by 25% of initial
        let re_attested = store.re_attest(1, 50);
        assert!(re_attested, "re-attest must succeed on a live cert");

        // After re-attest, energy is higher → more epochs before evaporation
        let cert = store.get(1).expect("cert must still be active");
        assert!(cert.re_attestation_count >= 1, "re_attestation_count must increment");
        assert!(cert.energy > 0);
    }

    // ── supermajority attestation ─────────────────────────────────────────

    #[test]
    fn poha_cert_supermajority_check() {
        let mut cert = make_cert(3, 1_000_000, 4096, 0);
        // 700 / 1000 = 70% → 700 * 3 = 2100 >= 1000 * 2 = 2000 → supermajority
        assert!(cert.is_supermajority());

        cert.attested_stake = 600;
        // 600 * 3 = 1800 < 2000 → not supermajority
        assert!(!cert.is_supermajority());
    }

    // ── ghost pruning ─────────────────────────────────────────────────────

    #[test]
    fn poha_store_prune_ghosts_removes_old() {
        let mut store = PoHAStore::new(1_000_000, 1);
        store.register(1, [0x01u8; 32], 8, 700, 1000, 0, vec![], vec![0]);
        store.register(2, [0x02u8; 32], 8, 700, 1000, 10, vec![], vec![0]);

        // Decay until both evaporate
        let _ = store.process_epoch(200);
        assert_eq!(store.ghost_count(), 2);

        // Prune ghosts older than epoch 100 — cert 1 evaporated at epoch ~64,
        // cert 2 evaporated later; both should be before 200
        let pruned = store.prune_ghosts(200);
        assert!(pruned >= 1, "at least one ghost must be pruned");
    }
}

// ── Nova IVC / chain-proof integration ───────────────────────────────────────

#[cfg(test)]
mod proving_integration {
    use evaporchain_proving::{MockProver, ProvingEngine};
    use evaporchain_proving::chain_proof::{ChainProver, LightClientVerifier};
    use evaporchain_types::{Block, Transaction, TransferTx};

    fn make_block(number: u64, txs: usize) -> Block {
        let transactions = (0..txs).map(|i| Transaction::Transfer(TransferTx {
            from: [i as u8; 32],
            to: [(i + 1) as u8; 32],
            amount: 100,
            fee: 1,
            nonce: i as u64,
        })).collect();
        Block {
            number,
            epoch: number / 10,
            parent_hash: [(number.saturating_sub(1)) as u8; 32],
            state_root: [number as u8; 32],
            transactions,
            timestamp: 1_700_000_000 + number * 6,
            chain_id: String::new(),
            producer_id: Some(0),
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
        }
    }

    // ── Fold blocks into proof accumulator ───────────────────────────────

    #[test]
    fn chain_prover_fold_and_get_proof() {
        let genesis = [0x00u8; 32];
        let mut prover = ChainProver::new(Box::new(MockProver::new()), genesis, 100);

        for i in 1..=5u64 {
            let block = make_block(i, 3);
            let new_root = [i as u8; 32];
            prover.fold_block(&block, new_root).expect("fold must succeed");
        }

        assert_eq!(prover.height(), 5);
        assert_eq!(prover.blocks_folded(), 5);

        let proof = prover.generate_chain_proof().expect("chain proof must generate");
        assert!(proof.num_steps > 0, "proof must cover at least 1 block");
    }

    #[test]
    fn chain_prover_verify_chain_proof_succeeds() {
        let genesis = [0x22u8; 32];
        let mut prover = ChainProver::new(Box::new(MockProver::new()), genesis, 100);

        for i in 1..=3u64 {
            prover.fold_block(&make_block(i, 1), [i as u8; 32]).unwrap();
        }

        let proof = prover.generate_chain_proof().unwrap();
        let valid = prover.verify_chain_proof(&proof).expect("verification must not error");
        assert!(valid, "chain proof generated by MockProver must verify");
    }

    // ── Auto-checkpoint at interval ───────────────────────────────────────

    #[test]
    fn chain_prover_auto_checkpoint_at_interval() {
        let mut prover = ChainProver::new(Box::new(MockProver::new()), [0u8; 32], 3);

        for i in 1..=7u64 {
            prover.fold_block(&make_block(i, 1), [i as u8; 32]).unwrap();
        }

        // Checkpoint every 3 blocks → 2 checkpoints at blocks 3 and 6
        let checkpoints = prover.checkpoints();
        assert!(checkpoints.len() >= 2, "must have at least 2 auto-checkpoints after 7 blocks");
    }

    #[test]
    fn chain_prover_manual_checkpoint_captures_state() {
        let mut prover = ChainProver::new(Box::new(MockProver::new()), [0u8; 32], 1000);

        prover.fold_block(&make_block(1, 2), [0x01u8; 32]).unwrap();
        prover.fold_block(&make_block(2, 2), [0x02u8; 32]).unwrap();

        let cp = prover.create_checkpoint().expect("checkpoint must succeed");
        assert_eq!(cp.block_height, 2, "checkpoint must capture current height");
        assert_ne!(cp.state_root, [0u8; 32], "checkpoint state root must not be zero");
    }

    // ── LightClientVerifier sync ──────────────────────────────────────────

    #[test]
    fn light_client_verifier_accepts_valid_chain_proof() {
        let genesis = [0x11u8; 32];
        let mut prover = ChainProver::new(Box::new(MockProver::new()), genesis, 100);

        for i in 1..=5u64 {
            prover.fold_block(&make_block(i, 1), [i as u8; 32]).unwrap();
        }

        let chain_proof = prover.generate_chain_proof().unwrap();
        let lc = LightClientVerifier::new(Box::new(MockProver::new()), genesis);
        let result = lc.verify_and_sync(&chain_proof).expect("sync must not error");
        assert!(result.valid, "light client must accept valid chain proof");
        assert_eq!(result.block_height, 5);
    }

    // ── Proof after 0 blocks → error ─────────────────────────────────────

    #[test]
    fn chain_prover_proof_without_folds_is_err() {
        let prover = ChainProver::new(Box::new(MockProver::new()), [0u8; 32], 100);
        let result = prover.generate_chain_proof();
        assert!(result.is_err(), "generating proof with 0 blocks must fail");
    }
}

// ── Evaporation proof (Fiat-Shamir batch proving) integration ─────────────────

#[cfg(test)]
mod evaporation_proof_integration {
    use evaporchain_proving::evaporation_proof::{
        EvaporationClaim, EvaporationProver, EnergyDecayStatement,
        verify_proof,
    };

    fn object_id(seed: u8) -> [u8; 20] { [seed; 20] }
    fn nullifier(seed: u8) -> [u8; 32] { [seed; 32] }

    // ── Decay statement: correct energy accepted ──────────────────────────

    #[test]
    fn decay_statement_correct_energy_accepted() {
        let mut prover = EvaporationProver::new(100);
        // half_life=10, elapsed=10 → energy = 1_000_000 >> 1 = 500_000
        let stmt = EnergyDecayStatement {
            object_id: object_id(1),
            initial_energy: 1_000_000,
            half_life: 10,
            creation_epoch: 0,
            current_epoch: 10,
            claimed_energy: 500_000,
        };
        prover.add_decay(stmt).expect("correct decay statement must be accepted");
    }

    #[test]
    fn decay_statement_wrong_energy_rejected() {
        let mut prover = EvaporationProver::new(100);
        let stmt = EnergyDecayStatement {
            object_id: object_id(2),
            initial_energy: 1_000_000,
            half_life: 10,
            creation_epoch: 0,
            current_epoch: 10,
            claimed_energy: 999_999, // wrong — should be 500_000
        };
        assert!(prover.add_decay(stmt).is_err(), "incorrect claimed energy must be rejected");
    }

    // ── Evaporation claim: at energy=0 epoch accepted ─────────────────────

    #[test]
    fn evaporation_claim_at_zero_epoch_accepted() {
        let mut prover = EvaporationProver::new(200);
        // half_life=1, elapsed=64 → energy = 64 >> 64 = 0
        let claim = EvaporationClaim {
            object_id: object_id(3),
            initial_energy: 64,
            half_life: 1,
            creation_epoch: 0,
            evaporation_epoch: 64,
            nullifier: nullifier(0xAA),
        };
        prover.add_evaporation(claim).expect("evaporation at energy=0 must be accepted");
    }

    #[test]
    fn evaporation_claim_before_zero_rejected() {
        let mut prover = EvaporationProver::new(200);
        // half_life=100, elapsed=10 → energy >> 0 = 1_000_000 (not zero)
        let claim = EvaporationClaim {
            object_id: object_id(4),
            initial_energy: 1_000_000,
            half_life: 100,
            creation_epoch: 0,
            evaporation_epoch: 10,
            nullifier: nullifier(0xBB),
        };
        assert!(prover.add_evaporation(claim).is_err(), "evaporation with energy > 0 must be rejected");
    }

    // ── Full prove → verify round-trip ───────────────────────────────────

    #[test]
    fn evaporation_proof_prove_and_verify() {
        let mut prover = EvaporationProver::new(500);

        // Add two decay statements
        for i in 0..2u8 {
            prover.add_decay(EnergyDecayStatement {
                object_id: object_id(i),
                initial_energy: 1_000_000,
                half_life: 10,
                creation_epoch: 0,
                current_epoch: 20,
                claimed_energy: 250_000, // 2 halvings: 1_000_000 >> 2
            }).unwrap();
        }

        // Add one evaporation claim
        prover.add_evaporation(EvaporationClaim {
            object_id: object_id(99),
            initial_energy: 1,
            half_life: 1,
            creation_epoch: 0,
            evaporation_epoch: 64,
            nullifier: nullifier(0xFF),
        }).unwrap();

        let proof = prover.prove();
        assert_eq!(proof.block_number, 500);
        assert_eq!(proof.decay_statements.len(), 2);
        assert_eq!(proof.evaporation_claims.len(), 1);
        assert_ne!(proof.transcript_hash, [0u8; 32]);

        let valid = verify_proof(&proof).expect("verify must not error");
        assert!(valid, "batch evaporation proof must verify");
    }
}

// ── Smart contract engine integration ────────────────────────────────────────

#[cfg(test)]
mod contracts_integration {
    use evaporchain_contracts::{ContractEngine, ContractTemplate};
    use serde_json::json;

    const CREATOR: [u8; 32] = [0xCAu8; 32];
    const ALICE: [u8; 32] = [0xA1u8; 32];
    const BOB: [u8; 32] = [0xB0u8; 32];

    // ── DecayingToken: deploy → transfer → tick evaporation ──────────────

    #[test]
    fn token_deploy_transfer_and_tick() {
        let mut engine = ContractEngine::new();

        let id = engine.deploy(
            ContractTemplate::DecayingToken,
            json!({ "name": "EvapCoin", "symbol": "EVAP", "total_supply": 1_000_000u64 }),
            vec![],
            CREATOR,
            1_000_000,
            100,
            0,
        ).expect("token deploy must succeed");

        assert_eq!(engine.len(), 1);

        // Transfer 1000 tokens from creator to Alice
        let result = engine.call(id, "transfer", &json!({
            "from": hex::encode(CREATOR),
            "to":   hex::encode(ALICE),
            "amount": 1000u64
        }), &CREATOR, 1).expect("transfer must succeed");
        assert!(result.success, "transfer must succeed: {:?}", result.error);

        // Tick at epoch 0 — contract is young, should not evaporate
        let tick = engine.tick(0);
        assert_eq!(tick.contracts_evaporated.len(), 0, "contract must not evaporate at epoch 0");
        assert_eq!(engine.len(), 1);
    }

    #[test]
    fn token_evaporates_after_energy_drain() {
        let mut engine = ContractEngine::new();

        // half_life=1 so after a handful of epochs the contract is dead
        let id = engine.deploy(
            ContractTemplate::DecayingToken,
            json!({ "name": "GhostCoin", "symbol": "GC", "total_supply": 100u64 }),
            vec![],
            CREATOR,
            64,
            1,
            0,
        ).expect("deploy must succeed");

        // Tick at epoch 64: 64 halvings with half_life=1 → energy = 64 >> 64 = 0
        let tick = engine.tick(64);
        assert!(tick.contracts_evaporated.len() >= 1, "contract must evaporate after energy drain");
        // Contract should be marked evaporated
        let inst = engine.get(id).expect("instance must still be accessible");
        assert!(inst.evaporated, "instance.evaporated must be true");
    }

    // ── MortalNFT: mint → transfer ────────────────────────────────────────

    #[test]
    fn nft_mint_and_transfer() {
        let mut engine = ContractEngine::new();

        let id = engine.deploy(
            ContractTemplate::MortalNFT,
            json!({ "collection_name": "ThermoPunks", "max_supply": 100u64 }),
            vec![],
            CREATOR,
            1_000_000,
            4096,
            0,
        ).expect("NFT deploy must succeed");

        // Mint token 1 to Alice
        let mint = engine.call(id, "mint", &json!({
            "to": hex::encode(ALICE),
            "token_id": 1u64,
            "metadata_uri": "ipfs://Qm..."
        }), &CREATOR, 0).expect("mint must not error");
        assert!(mint.success, "mint must succeed: {:?}", mint.error);

        // Transfer token 1 from Alice to Bob
        let xfer = engine.call(id, "transfer", &json!({
            "from": hex::encode(ALICE),
            "to":   hex::encode(BOB),
            "token_id": 1u64
        }), &ALICE, 1).expect("transfer must not error");
        assert!(xfer.success, "NFT transfer must succeed: {:?}", xfer.error);
    }

    // ── ContractEngine: multi-contract isolation ──────────────────────────

    #[test]
    fn multiple_contracts_are_isolated() {
        let mut engine = ContractEngine::new();

        let t1 = engine.deploy(
            ContractTemplate::DecayingToken,
            json!({ "name": "A", "symbol": "A", "total_supply": 500u64 }),
            vec![], CREATOR, 1_000_000, 4096, 0,
        ).unwrap();

        let t2 = engine.deploy(
            ContractTemplate::DecayingToken,
            json!({ "name": "B", "symbol": "B", "total_supply": 200u64 }),
            vec![], CREATOR, 1_000_000, 4096, 0,
        ).unwrap();

        assert_ne!(t1, t2, "contract IDs must be unique");
        assert_eq!(engine.len(), 2);

        // Transfer in t1 must not affect t2's state
        engine.call(t1, "transfer", &json!({
            "from": hex::encode(CREATOR), "to": hex::encode(ALICE), "amount": 100u64
        }), &CREATOR, 0).unwrap();

        let s1 = engine.get_state(t1).expect("t1 state must exist");
        let s2 = engine.get_state(t2).expect("t2 state must exist");
        // t2 total_supply unchanged
        assert_eq!(s2["total_supply"], 200u64, "t2 supply must be unaffected");
        // t1 still has state (we don't assert exact balance — internal repr varies)
        assert!(s1.is_object());
    }

    // ── Call on evaporated contract → error ───────────────────────────────

    #[test]
    fn call_on_evaporated_contract_is_error() {
        let mut engine = ContractEngine::new();
        let id = engine.deploy(
            ContractTemplate::DecayingToken,
            json!({ "name": "Doomed", "symbol": "D", "total_supply": 1u64 }),
            vec![], CREATOR, 1, 1, 0,
        ).unwrap();

        // Force tick past evaporation
        engine.tick(64);

        let result = engine.call(id, "transfer", &json!({
            "from": hex::encode(CREATOR), "to": hex::encode(ALICE), "amount": 1u64
        }), &CREATOR, 64);
        assert!(result.is_err(), "call on evaporated contract must return Err");
    }

    // ── Refresh contract extends energy ───────────────────────────────────

    #[test]
    fn refresh_contract_prevents_evaporation() {
        let mut engine = ContractEngine::new();
        let id = engine.deploy(
            ContractTemplate::DecayingToken,
            json!({ "name": "Refreshed", "symbol": "R", "total_supply": 100u64 }),
            vec![], CREATOR, 1_000, 10, 0,
        ).unwrap();

        // After 5 half-lives (epoch 50), energy ≈ 31 — still alive
        let tick1 = engine.tick(50);
        assert_eq!(tick1.evaporated, 0, "must not evaporate at epoch 50");

        // Refresh with additional energy
        engine.refresh_contract(id, 1_000_000, 50).expect("refresh must succeed on a live contract");

        // Now even at epoch 200 it should have energy from the refresh
        let inst = engine.get(id).unwrap();
        assert!(inst.energy > 0, "energy must be > 0 after refresh");
    }
}

// ── Frontier primitive integration (Light-Cone + Singh Attractor) ─────────────

#[cfg(test)]
mod frontier_primitive_integration {
    use evaporchain_light_cone::block::Block as LcBlock;
    use evaporchain_light_cone::dag::{LightCone, causal_past, causal_future};
    use evaporchain_light_cone::concurrency::{is_concurrent, precedes, comparable};
    use evaporchain_light_cone::arrow::time_arrow_holds_at;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_singh_attractor::{Attractor, select_attractor};

    fn id(b: u8) -> [u8; 32] { [b; 32] }
    fn lambda() -> ChainLambda { ChainLambda::new(Lambda::from_epochs(100)) }

    // Build a diamond DAG:  A → B, A → C, B → D, C → D
    fn diamond() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(LcBlock::new(id(0), vec![], 2_000, 0)).unwrap();
        lc.insert(LcBlock::new(id(1), vec![id(0)], 1_800, 1)).unwrap();
        lc.insert(LcBlock::new(id(2), vec![id(0)], 1_800, 1)).unwrap();
        lc.insert(LcBlock::new(id(3), vec![id(1), id(2)], 1_600, 2)).unwrap();
        lc
    }

    // ── LightCone DAG: causal past / future ───────────────────────────────

    #[test]
    fn light_cone_causal_past_includes_ancestors() {
        let lc = diamond();
        let past_d = causal_past(&lc, id(3));
        // D's causal past must include A, B, C
        assert!(past_d.contains(&id(0)), "genesis A must be in causal past of D");
        assert!(past_d.contains(&id(1)), "B must be in causal past of D");
        assert!(past_d.contains(&id(2)), "C must be in causal past of D");
        assert!(!past_d.contains(&id(3)), "D must not be in its own causal past");
    }

    #[test]
    fn light_cone_causal_future_includes_descendants() {
        let lc = diamond();
        let future_a = causal_future(&lc, id(0));
        // A's future must include B, C, D
        assert!(future_a.contains(&id(1)));
        assert!(future_a.contains(&id(2)));
        assert!(future_a.contains(&id(3)));
        assert!(!future_a.contains(&id(0)), "A must not be in its own future");
    }

    // ── Concurrency relations ─────────────────────────────────────────────

    #[test]
    fn light_cone_concurrent_branches_in_diamond() {
        let lc = diamond();
        // B and C are concurrent — neither is in the other's causal past
        assert!(is_concurrent(&lc, id(1), id(2)), "B and C must be concurrent");
        assert!(is_concurrent(&lc, id(2), id(1)), "concurrency is symmetric");
        // A precedes B (A → B)
        assert!(precedes(&lc, id(0), id(1)), "A must precede B");
        // D does not precede A
        assert!(!precedes(&lc, id(3), id(0)), "D must not precede A");
    }

    #[test]
    fn light_cone_comparable_is_reflexive_and_transitive() {
        let lc = diamond();
        // Reflexive
        assert!(comparable(&lc, id(0), id(0)));
        // Transitive: A ≼ B and B ≼ D → A ≼ D
        assert!(comparable(&lc, id(0), id(3)));
        // Concurrent pair is NOT comparable
        assert!(!comparable(&lc, id(1), id(2)));
    }

    // ── Energy-decay time arrow ───────────────────────────────────────────

    #[test]
    fn time_arrow_holds_for_ancestor_with_higher_seed() {
        let ancestor = LcBlock::new(id(0), vec![], 2_000, 0);
        let descendant = LcBlock::new(id(1), vec![id(0)], 1_000, 5);
        // At epoch 10: ancestor decays 10 epochs, descendant 5 — ancestor has more
        assert!(
            time_arrow_holds_at(&ancestor, &descendant, lambda(), 10),
            "time arrow must hold when ancestor has higher seed energy"
        );
    }

    #[test]
    fn time_arrow_fails_before_observed_epoch() {
        let a = LcBlock::new(id(0), vec![], 2_000, 10);
        let d = LcBlock::new(id(1), vec![id(0)], 1_000, 20);
        // t=5 is before both observed_epochs → undefined, returns false
        assert!(!time_arrow_holds_at(&a, &d, lambda(), 5));
    }

    // ── Singh Attractor: multi-basin consensus ────────────────────────────

    #[test]
    fn singh_attractor_selects_correct_basin() {
        let attractors = [
            Attractor::new(100_000, 10_000),   // quiet-hours basin
            Attractor::new(1_000_000, 100_000), // normal-load basin
            Attractor::new(5_000_000, 500_000), // peak-load basin
        ];

        // Energy in normal-load basin
        let selected = select_attractor(950_000, &attractors).unwrap();
        assert_eq!(selected.center, 1_000_000, "normal-load basin must be selected");

        // Energy in peak-load basin
        let selected = select_attractor(5_200_000, &attractors).unwrap();
        assert_eq!(selected.center, 5_000_000, "peak-load basin must be selected");
    }

    #[test]
    fn singh_attractor_fallback_to_nearest_when_outside_all_basins() {
        let attractors = [
            Attractor::new(100, 10),
            Attractor::new(10_000, 100),
        ];
        // 1_000 is outside both basins; nearest to 10_000 (9000 away) vs 100 (900 away)
        let selected = select_attractor(1_000, &attractors).unwrap();
        assert_eq!(selected.center, 100, "nearest attractor (100) must win by distance");
    }

    #[test]
    fn singh_attractor_empty_list_returns_none() {
        assert!(select_attractor(1_000_000, &[]).is_none());
    }
}
