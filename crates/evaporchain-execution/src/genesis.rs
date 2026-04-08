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
        };
        db.put_account(account);
        total_allocated += ga.balance;
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
        };
        db.put_object(obj);
    }

    // 4. Compute state root
    let state_root = db.compute_state_root();

    // 5. Build genesis block
    let genesis_block = Block {
        number: 0,
        epoch: 0,
        parent_hash: [0u8; 32], // no parent
        state_root,
        transactions: vec![],
        timestamp: 0,
        producer_id: None,
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
            },
            genesis_time: "2026-01-01T00:00:00Z".into(),
            validators: vec![GenesisValidator {
                id: 1,
                name: "val-1".into(),
                stake: 1000,
                address: addr(1),
                bls_public_key: None,
                p2p_address: None,
            }],
            accounts: vec![
                GenesisAccount {
                    address: addr(1),
                    balance: 500_000,
                    label: "Validator-1".into(),
                },
                GenesisAccount {
                    address: addr(2),
                    balance: 300_000,
                    label: "Foundation".into(),
                },
            ],
            objects: vec![],
            bootstrap_peers: vec![],
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
}
