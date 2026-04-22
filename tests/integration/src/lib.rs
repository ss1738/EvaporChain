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
