/// Repair tool: read or patch the CF_META/parent_hash key in a ChainStore RocksDB.
///
/// Usage:
///   repair-meta --data-dir <path> --read
///   repair-meta --data-dir <path> --write <64-char-hex>
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::env;

const CF_META: &str = "chain_meta";

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut data_dir: Option<String> = None;
    let mut mode = "read";
    let mut new_hash_hex: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).cloned();
            }
            "--read" => mode = "read",
            "--write" => {
                mode = "write";
                i += 1;
                new_hash_hex = args.get(i).cloned();
            }
            _ => {}
        }
        i += 1;
    }

    let data_dir = data_dir.unwrap_or_else(|| {
        eprintln!("Usage: repair-meta --data-dir <path> --read");
        eprintln!("       repair-meta --data-dir <path> --write <64-char-hex>");
        std::process::exit(1);
    });

    let chain_path = format!("{}/chain", data_dir);
    println!("Opening chain store at: {}", chain_path);

    // List all existing column families so we open them all correctly
    let cf_names = DB::list_cf(&Options::default(), &chain_path).unwrap_or_else(|_| {
        vec!["default".to_string(), CF_META.to_string()]
    });
    println!("Column families: {:?}", cf_names);

    let cf_descriptors: Vec<ColumnFamilyDescriptor> = cf_names
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(name.as_str(), Options::default()))
        .collect();

    let mut opts = Options::default();
    opts.create_if_missing(false);
    opts.create_missing_column_families(false);

    let db = DB::open_cf_descriptors(&opts, &chain_path, cf_descriptors).unwrap_or_else(|e| {
        eprintln!("Failed to open RocksDB: {}", e);
        std::process::exit(1);
    });

    let cf = db.cf_handle(CF_META).unwrap_or_else(|| {
        eprintln!("CF_META ('{}') column family not found", CF_META);
        std::process::exit(1);
    });

    let block_number = db
        .get_cf(cf, b"block_number").ok().flatten()
        .map(|v| u64::from_le_bytes(v[..8].try_into().unwrap()))
        .unwrap_or(0);

    let epoch = db
        .get_cf(cf, b"epoch").ok().flatten()
        .map(|v| u64::from_le_bytes(v[..8].try_into().unwrap()))
        .unwrap_or(0);

    let current_hash = db
        .get_cf(cf, b"parent_hash").ok().flatten()
        .map(|v| hex::encode(&v[..32]))
        .unwrap_or_else(|| "none".to_string());

    println!("block_number : {}", block_number);
    println!("epoch        : {}", epoch);
    println!("parent_hash  : {}", current_hash);

    if mode == "write" {
        let hex_str = new_hash_hex.unwrap_or_else(|| {
            eprintln!("--write requires a 64-char hex string");
            std::process::exit(1);
        });
        if hex_str.len() != 64 {
            eprintln!("parent_hash must be 64 hex chars (32 bytes), got {}", hex_str.len());
            std::process::exit(1);
        }
        let new_hash = hex::decode(&hex_str).unwrap_or_else(|e| {
            eprintln!("Invalid hex: {}", e);
            std::process::exit(1);
        });
        db.put_cf(cf, b"parent_hash", &new_hash).unwrap_or_else(|e| {
            eprintln!("Failed to write parent_hash: {}", e);
            std::process::exit(1);
        });
        println!("parent_hash  -> {} (WRITTEN)", hex_str);
    }
}
