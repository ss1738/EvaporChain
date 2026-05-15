//! Genesis block builder.
//!
//! Initializes chain state from a `GenesisConfig`: creates accounts, objects,
//! sets up the validator set, and produces the genesis block (block 0, epoch 0).

use evaporchain_state::db::StateDB;
use evaporchain_types::genesis::GenesisConfig;
use evaporchain_types::{Account, Block, ObjectState, StateObject};

/// Result of genesis initialization.
#[derive(Debug)]
pub struct GenesisResult {
    /// The genesis block (block 0).
    pub block: Block,
    /// State root after genesis initialization.
    pub state_root: [u8; 32],
    /// Number of accounts created.
    pub accounts_created: usize,
    /// Number of objects created.
    pub objects_created: usize,
    /// Number of validators registered.
    pub validators_registered: usize,
    /// Total tokens allocated.
    pub total_allocated: u64,
}

/// Initialize chain state from a genesis configuration.
///
/// This function:
/// 1. Validates the genesis config
/// 2. Creates all genesis accounts with their initial balances
/// 3. Creates all genesis state objects
/// 4. Computes the genesis state root
/// 5. Returns the genesis block (block 0)
///
/// The validator set is returned via the config — the caller is responsible
/// for initializing the consensus layer with the genesis validators.
pub fn initialize_genesis(
    db: &mut dyn StateDB,
    config: &GenesisConfig,
) -> Result<GenesisResult, String> {
    // 1. Validate config
    config
        .validate()
        .map_err(|errors| format!("Invalid genesis config: {}", errors.join("; ")))?;

    // 2. Create genesis accounts
    let mut total_allocated = 0u64;
    for ga in &config.accounts {
        let account = Account {
            address: ga.address,
            balance: ga.balance,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: ga.vesting,
        };
        db.put_account(account);
        // GEN-N2 (audit 2026-05-15): checked_add — a coordinator-signed
        // genesis whose account balances sum past `u64::MAX` would wrap
        // `total_allocated` and break downstream emission-cap reads. The
        // sum can't overflow in any realistic tokenomics, but a
        // compromised coordinator constructing a poisoned genesis can
        // engineer it.
        total_allocated = total_allocated.checked_add(ga.balance).ok_or_else(|| {
            format!(
                "genesis: sum of account balances overflows u64 \
                 (last addition {} would overflow {})",
                ga.balance, total_allocated
            )
        })?;
    }

    // 3. Create genesis objects
    for go in &config.objects {
        let obj = StateObject {
            id: go.id,
            owner: go.owner,
            energy: go.energy,
            half_life: go.half_life,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: go.data.clone(),
            decay_curve: None,
            lad_mode: None,
        };
        db.put_object(obj);
    }

    // 4. Compute state root.
    //
    // GEN-N3 (audit 2026-05-15): bind the canonical genesis-config
    // hash into the state_root. Without this, only fields that the
    // pipeline actually writes into `db` (accounts, objects) end up
    // in the state_root — fields like `chain_id`, tokenomics,
    // validator set, coordinator pk, bootstrap peers, etc. don't.
    // Two configs that differ only in those fields can produce
    // identical state_roots and the chain silently forks at height
    // 0.  Binding `canonical_genesis_hash()` into state_root means
    // any divergence in any config field forces a different state
    // root at genesis, surfacing the misconfiguration immediately
    // at first cross-node block-sync rather than after the first
    // attestation.
    const GENESIS_BIND_DST: &[u8] = b"EVAPORCHAIN_V1_GENESIS_BIND\0";
    let raw_state_root = db.compute_state_root();
    let genesis_hash = config.canonical_genesis_hash();
    let mut bind_input =
        Vec::with_capacity(GENESIS_BIND_DST.len() + raw_state_root.len() + genesis_hash.len());
    bind_input.extend_from_slice(GENESIS_BIND_DST);
    bind_input.extend_from_slice(&raw_state_root);
    bind_input.extend_from_slice(&genesis_hash);
    let state_root = *blake3::hash(&bind_input).as_bytes();

    // 5. Build genesis block
    let genesis_block = Block {
        number: 0,
        epoch: 0,
        parent_hash: [0u8; 32], // no parent
        state_root,
        transactions: vec![],
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
        da_row_roots: vec![],
        da_col_roots: vec![],
        parents: vec![],
        post_state_root: None,
    };

    Ok(GenesisResult {
        block: genesis_block,
        state_root,
        accounts_created: config.accounts.len(),
        objects_created: config.objects.len(),
        validators_registered: config.validators.len(),
        total_allocated,
    })
}

/// Load a genesis config from a JSON string.
pub fn load_genesis_config(json: &str) -> Result<GenesisConfig, String> {
    serde_json::from_str(json).map_err(|e| format!("Failed to parse genesis JSON: {}", e))
}

// ─────────────────────── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::genesis::{
        ChainParams, GenesisAccount, GenesisConfig, GenesisObject, GenesisValidator, Tokenomics,
    };

    fn addr(byte: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = byte;
        a
    }

    fn obj_id(byte: u8) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = byte;
        id
    }

    fn minimal_config() -> GenesisConfig {
        GenesisConfig {
            chain_params: ChainParams::testnet(),
            tokenomics: Tokenomics {
                total_supply: 1_000_000,
                block_reward: 10,
                reward_half_life: 1000,
                fee_burn_rate: 0.5,
                staker_fee_share: 0.5,
                target_staking_apy: 0.05,
                validator_commission_default: Tokenomics::default_commission(),
                max_supply_cap: None,
                emission: None,
                blocks_per_year: Tokenomics::default_blocks_per_year(),
            },
            genesis_time: "2026-01-01T00:00:00Z".into(),
            validators: vec![GenesisValidator {
                id: 1,
                name: "val-1".into(),
                stake: 1000,
                address: addr(1),
                bls_public_key: None,
                bls_pop: None,
                p2p_address: None,
            }],
            accounts: vec![
                GenesisAccount {
                    address: addr(1),
                    balance: 500_000,
                    label: "Validator-1".into(),
                    vesting: None,
                },
                GenesisAccount {
                    address: addr(2),
                    balance: 300_000,
                    label: "Foundation".into(),
                    vesting: None,
                },
            ],
            objects: vec![],
            bootstrap_peers: vec![],
            trusted_checkpoint: None,
            coordinator_pk: None,
            coordinator_signature: None,
        }
    }

    #[test]
    fn test_genesis_creates_accounts() {
        let mut db = InMemoryStateDB::new();
        let config = minimal_config();
        let result = initialize_genesis(&mut db, &config).unwrap();

        assert_eq!(result.accounts_created, 2);
        assert_eq!(result.total_allocated, 800_000);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 500_000);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 300_000);
    }

    #[test]
    fn test_genesis_creates_objects() {
        let mut db = InMemoryStateDB::new();
        let mut config = minimal_config();
        config.objects.push(GenesisObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 10_000,
            half_life: 100,
            data: vec![0xAB; 32],
        });

        let result = initialize_genesis(&mut db, &config).unwrap();
        assert_eq!(result.objects_created, 1);

        let obj = db.get_object(&obj_id(1)).unwrap();
        assert_eq!(obj.energy, 10_000);
        assert_eq!(obj.half_life, 100);
        assert_eq!(obj.state, ObjectState::Active);
    }

    #[test]
    fn test_genesis_block_is_block_zero() {
        let mut db = InMemoryStateDB::new();
        let config = minimal_config();
        let result = initialize_genesis(&mut db, &config).unwrap();

        assert_eq!(result.block.number, 0);
        assert_eq!(result.block.epoch, 0);
        assert_eq!(result.block.parent_hash, [0u8; 32]);
        assert!(result.block.transactions.is_empty());
    }

    #[test]
    fn test_genesis_state_root_nonzero() {
        let mut db = InMemoryStateDB::new();
        let config = minimal_config();
        let result = initialize_genesis(&mut db, &config).unwrap();

        assert_ne!(result.state_root, [0u8; 32]);
    }

    #[test]
    fn test_genesis_state_root_deterministic() {
        let config = minimal_config();

        let mut db1 = InMemoryStateDB::new();
        let r1 = initialize_genesis(&mut db1, &config).unwrap();

        let mut db2 = InMemoryStateDB::new();
        let r2 = initialize_genesis(&mut db2, &config).unwrap();

        assert_eq!(r1.state_root, r2.state_root);
    }

    #[test]
    fn test_genesis_rejects_invalid_config() {
        let mut db = InMemoryStateDB::new();
        let mut config = minimal_config();
        config.validators.clear();

        let result = initialize_genesis(&mut db, &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one validator"));
    }

    #[test]
    fn test_genesis_json_roundtrip() {
        let config = minimal_config();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed = load_genesis_config(&json).unwrap();
        assert_eq!(parsed.chain_params.chain_id, config.chain_params.chain_id);
    }

    #[test]
    fn test_genesis_validators_counted() {
        let mut db = InMemoryStateDB::new();
        let mut config = minimal_config();
        config.validators.push(GenesisValidator {
            id: 2,
            name: "val-2".into(),
            stake: 2000,
            address: addr(3),
            bls_public_key: None,
            bls_pop: None,
            p2p_address: None,
        });

        let result = initialize_genesis(&mut db, &config).unwrap();
        assert_eq!(result.validators_registered, 2);
    }

    #[test]
    fn test_testnet_default_config_valid() {
        let config = GenesisConfig::testnet_default();
        assert!(config.validate().is_ok());

        let mut db = InMemoryStateDB::new();
        let result = initialize_genesis(&mut db, &config).unwrap();
        assert_eq!(result.accounts_created, 3);
        assert_eq!(result.validators_registered, 2);
    }

    // ─── GEN-N3 (audit 2026-05-15): canonical genesis hash binds into state_root ───

    /// GEN-N3: changing `chain_id` (a field that is NOT otherwise
    /// written into state) MUST change the genesis `state_root`.
    /// Pre-fix, two nodes with diverging chain_ids but identical
    /// account allocations produced the same state_root and silently
    /// forked at height 0.
    #[test]
    fn gen_n3_state_root_depends_on_chain_id() {
        let mut db_a = InMemoryStateDB::new();
        let mut cfg_a = minimal_config();
        cfg_a.chain_params.chain_id = "evaporchain-testnet-A".into();
        let r_a = initialize_genesis(&mut db_a, &cfg_a).unwrap();

        let mut db_b = InMemoryStateDB::new();
        let mut cfg_b = minimal_config();
        cfg_b.chain_params.chain_id = "evaporchain-testnet-B".into();
        let r_b = initialize_genesis(&mut db_b, &cfg_b).unwrap();

        assert_ne!(
            r_a.state_root, r_b.state_root,
            "state_root MUST diverge when chain_id diverges"
        );
    }

    /// GEN-N3: changing validator stake (registered separately in
    /// consensus, not in state DB) MUST change state_root.
    #[test]
    fn gen_n3_state_root_depends_on_validator_stake() {
        let mut db_a = InMemoryStateDB::new();
        let cfg_a = minimal_config();
        let r_a = initialize_genesis(&mut db_a, &cfg_a).unwrap();

        let mut db_b = InMemoryStateDB::new();
        let mut cfg_b = minimal_config();
        cfg_b.validators[0].stake = cfg_a.validators[0].stake + 1;
        let r_b = initialize_genesis(&mut db_b, &cfg_b).unwrap();

        assert_ne!(
            r_a.state_root, r_b.state_root,
            "state_root MUST diverge when a validator's stake diverges"
        );
    }

    /// GEN-N3: changing tokenomics (e.g. block_reward, never written
    /// to state) MUST change state_root.
    #[test]
    fn gen_n3_state_root_depends_on_tokenomics() {
        let mut db_a = InMemoryStateDB::new();
        let cfg_a = minimal_config();
        let r_a = initialize_genesis(&mut db_a, &cfg_a).unwrap();

        let mut db_b = InMemoryStateDB::new();
        let mut cfg_b = minimal_config();
        cfg_b.tokenomics.block_reward = cfg_a.tokenomics.block_reward + 1;
        let r_b = initialize_genesis(&mut db_b, &cfg_b).unwrap();

        assert_ne!(
            r_a.state_root, r_b.state_root,
            "state_root MUST diverge when tokenomics diverge"
        );
    }

    /// GEN-N3: changing `genesis_time` MUST change state_root.  The
    /// only field that ties the chain to a specific launch event.
    #[test]
    fn gen_n3_state_root_depends_on_genesis_time() {
        let mut db_a = InMemoryStateDB::new();
        let cfg_a = minimal_config();
        let r_a = initialize_genesis(&mut db_a, &cfg_a).unwrap();

        let mut db_b = InMemoryStateDB::new();
        let mut cfg_b = minimal_config();
        cfg_b.genesis_time = "2030-01-01T00:00:00Z".into();
        let r_b = initialize_genesis(&mut db_b, &cfg_b).unwrap();

        assert_ne!(
            r_a.state_root, r_b.state_root,
            "state_root MUST diverge when genesis_time diverges"
        );
    }

    /// GEN-N3: changing `bootstrap_peers` MUST change state_root.
    /// Less critical doctrinally, but a config-divergence signal
    /// the operator probably wants surfaced.
    #[test]
    fn gen_n3_state_root_depends_on_bootstrap_peers() {
        let mut db_a = InMemoryStateDB::new();
        let cfg_a = minimal_config();
        let r_a = initialize_genesis(&mut db_a, &cfg_a).unwrap();

        let mut db_b = InMemoryStateDB::new();
        let mut cfg_b = minimal_config();
        cfg_b.bootstrap_peers.push("/ip4/10.0.0.1/tcp/9000".into());
        let r_b = initialize_genesis(&mut db_b, &cfg_b).unwrap();

        assert_ne!(
            r_a.state_root, r_b.state_root,
            "state_root MUST diverge when bootstrap_peers diverge"
        );
    }

    /// GEN-N3: identical configs MUST still produce identical
    /// state_roots (determinism preserved).  Existing
    /// `test_genesis_state_root_deterministic` covers the minimal
    /// case; this is the post-fix sanity that nothing flake-y was
    /// introduced.
    #[test]
    fn gen_n3_state_root_deterministic_under_full_config() {
        let cfg = minimal_config();
        let mut db1 = InMemoryStateDB::new();
        let r1 = initialize_genesis(&mut db1, &cfg).unwrap();
        let mut db2 = InMemoryStateDB::new();
        let r2 = initialize_genesis(&mut db2, &cfg).unwrap();
        assert_eq!(r1.state_root, r2.state_root);
        // And the genesis_hash standalone is also deterministic.
        assert_eq!(cfg.canonical_genesis_hash(), cfg.canonical_genesis_hash());
    }

    /// GEN-N3: `canonical_genesis_hash` is DST-prefixed, so its
    /// output cannot collide with any unrelated BLAKE3 invocation
    /// in the protocol (e.g., a hash of the same JSON bytes that
    /// happens to land in another context).
    #[test]
    fn gen_n3_canonical_genesis_hash_uses_dst_prefix() {
        let cfg = minimal_config();
        let raw_signing = cfg.canonical_signing_bytes();
        let raw_blake = *blake3::hash(&raw_signing).as_bytes();
        // Without the DST, the two would coincide.
        assert_ne!(
            raw_blake,
            cfg.canonical_genesis_hash(),
            "canonical_genesis_hash MUST not equal BLAKE3 of the bare signing bytes"
        );
    }

    /// Regression test for TOKENOMICS §2.6 genesis-mainnet.json placeholder
    /// vesting schedules. Loads the actual repo file and asserts that the
    /// 7 vesting-attached buckets carry their schedules through to chain
    /// state. If this fails, either the genesis JSON drifted from the
    /// expected layout OR the GenesisAccount.vesting → Account.vesting
    /// carry-through in initialize_genesis broke.
    #[test]
    fn test_mainnet_genesis_applies_vesting() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("genesis-mainnet.json");
        if !path.exists() {
            // Repo layout assumption — skip silently if not findable.
            return;
        }
        let json = std::fs::read_to_string(&path).expect("read genesis-mainnet.json");
        let config = load_genesis_config(&json).expect("parse genesis-mainnet.json");

        // Sanity: total supply matches sum of balances.
        let total_balance: u64 = config.accounts.iter().map(|a| a.balance).sum();
        assert_eq!(total_balance, config.tokenomics.total_supply);

        // 7 of 8 buckets vested (Community Airdrop is the only liquid one).
        let vested_count = config.accounts.iter().filter(|a| a.vesting.is_some()).count();
        let liquid_count = config.accounts.iter().filter(|a| a.vesting.is_none()).count();
        assert_eq!(vested_count, 7, "expected 7 vested accounts");
        assert_eq!(liquid_count, 1, "expected 1 liquid account (Airdrop)");

        // Day-one liquid = total − sum(total_locked).
        let total_locked: u64 = config.accounts.iter()
            .filter_map(|a| a.vesting.as_ref().map(|v| v.total_locked))
            .sum();
        assert_eq!(total_locked, 900_000_000, "90% of supply must be locked");

        // Carry-through: initialize a chain and verify the Account
        // records on-chain have the vesting field populated.
        let mut db = InMemoryStateDB::new();
        initialize_genesis(&mut db, &config).expect("initialize_genesis");
        for ga in &config.accounts {
            let acc = db.get_account(&ga.address)
                .unwrap_or_else(|| panic!("account {} not in db", ga.label));
            assert_eq!(acc.balance, ga.balance);
            assert_eq!(acc.vesting, ga.vesting,
                "vesting on {} did not survive genesis init", ga.label);

            // Pre-cliff: locked balance equals total_locked.
            if let Some(v) = &acc.vesting {
                assert_eq!(acc.transferable_balance(0), 0,
                    "{} pre-cliff transferable must be 0", ga.label);
                assert_eq!(acc.transferable_balance(v.cliff_epoch), 0,
                    "{} at cliff_epoch transferable must still be 0", ga.label);
                // Post full release: transferable = balance.
                let post_release = v.cliff_epoch + v.linear_release_epochs + 1;
                assert_eq!(acc.transferable_balance(post_release), acc.balance,
                    "{} post-release transferable must equal balance", ga.label);
            }
        }
    }
}
