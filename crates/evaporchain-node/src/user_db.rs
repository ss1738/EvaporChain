//! User database backed by SQLite — accounts, wallets, activity logs.

use rusqlite::{Connection, params};
use std::sync::Mutex;

pub struct UserDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub display_name: String,
    pub created_at: String,
    pub email_verified: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Wallet {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub address: String,
    pub public_key: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActivityEntry {
    pub id: i64,
    pub wallet_address: String,
    pub action: String,
    pub details: String,
    pub timestamp: String,
}

impl UserDb {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("DB open: {e}"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| format!("Pragma: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                display_name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_login TEXT,
                email_verified INTEGER DEFAULT 0,
                verification_code TEXT
            );

            CREATE TABLE IF NOT EXISTS wallets (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL,
                name TEXT DEFAULT 'Main Wallet',
                address TEXT UNIQUE NOT NULL,
                public_key TEXT NOT NULL,
                encrypted_private_key TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (user_id) REFERENCES users(id)
            );

            CREATE TABLE IF NOT EXISTS activity (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL,
                wallet_address TEXT NOT NULL,
                action TEXT NOT NULL,
                details TEXT,
                timestamp TEXT NOT NULL,
                FOREIGN KEY (user_id) REFERENCES users(id)
            );"
        ).map_err(|e| format!("Schema: {e}"))?;

        Ok(Self { conn: Mutex::new(conn) })
    }

    // ── Users ──

    pub fn create_user(&self, email: &str, password_hash: &str, display_name: &str, verification_code: &str) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO users (email, password_hash, display_name, created_at, verification_code) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![email, password_hash, display_name, now, verification_code],
        ).map_err(|e| {
            if e.to_string().contains("UNIQUE") { "Email already registered".into() }
            else { format!("Create user: {e}") }
        })?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_user_by_email(&self, email: &str) -> Result<Option<(i64, String, String, bool, Option<String>)>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, password_hash, display_name, email_verified, verification_code FROM users WHERE email = ?1"
        ).map_err(|e| format!("{e}"))?;
        let result = stmt.query_row(params![email], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        });
        match result {
            Ok(u) => Ok(Some(u)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("{e}")),
        }
    }

    pub fn get_user_by_id(&self, id: i64) -> Result<Option<User>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, email, display_name, created_at, email_verified FROM users WHERE id = ?1"
        ).map_err(|e| format!("{e}"))?;
        let result = stmt.query_row(params![id], |row| {
            Ok(User {
                id: row.get(0)?,
                email: row.get(1)?,
                display_name: row.get(2)?,
                created_at: row.get(3)?,
                email_verified: row.get(4)?,
            })
        });
        match result {
            Ok(u) => Ok(Some(u)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("{e}")),
        }
    }

    pub fn verify_email(&self, email: &str, code: &str) -> Result<bool, String> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE users SET email_verified = 1 WHERE email = ?1 AND verification_code = ?2 AND email_verified = 0",
            params![email, code],
        ).map_err(|e| format!("{e}"))?;
        Ok(affected > 0)
    }

    pub fn update_last_login(&self, user_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute("UPDATE users SET last_login = ?1 WHERE id = ?2", params![now, user_id])
            .map_err(|e| format!("{e}"))?;
        Ok(())
    }

    // ── Wallets ──

    pub fn create_wallet(&self, user_id: i64, name: &str, address: &str, public_key: &str, encrypted_private_key: &str) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO wallets (user_id, name, address, public_key, encrypted_private_key, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![user_id, name, address, public_key, encrypted_private_key, now],
        ).map_err(|e| {
            if e.to_string().contains("UNIQUE") { "Address already exists".into() }
            else { format!("Create wallet: {e}") }
        })?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_wallets(&self, user_id: i64) -> Result<Vec<Wallet>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, name, address, public_key, created_at FROM wallets WHERE user_id = ?1 ORDER BY id"
        ).map_err(|e| format!("{e}"))?;
        let rows = stmt.query_map(params![user_id], |row| {
            Ok(Wallet {
                id: row.get(0)?,
                user_id: row.get(1)?,
                name: row.get(2)?,
                address: row.get(3)?,
                public_key: row.get(4)?,
                created_at: row.get(5)?,
            })
        }).map_err(|e| format!("{e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("{e}"))
    }

    pub fn get_wallet_owner(&self, address: &str) -> Result<Option<i64>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT user_id FROM wallets WHERE address = ?1")
            .map_err(|e| format!("{e}"))?;
        match stmt.query_row(params![address], |row| row.get::<_, i64>(0)) {
            Ok(uid) => Ok(Some(uid)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("{e}")),
        }
    }

    /// Retrieve the public key and secret key (hex-encoded) for a wallet address.
    /// Returns None if the wallet doesn't exist.
    pub fn get_wallet_keys(&self, address: &str) -> Result<Option<(String, String)>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT public_key, encrypted_private_key FROM wallets WHERE address = ?1")
            .map_err(|e| format!("{e}"))?;
        match stmt.query_row(params![address], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            Ok(keys) => Ok(Some(keys)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("{e}")),
        }
    }

    // ── Activity ──

    pub fn log_activity(&self, user_id: i64, wallet_address: &str, action: &str, details: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO activity (user_id, wallet_address, action, details, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![user_id, wallet_address, action, details, now],
        ).map_err(|e| format!("{e}"))?;
        Ok(())
    }

    pub fn get_activity(&self, user_id: i64, limit: usize) -> Result<Vec<ActivityEntry>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, wallet_address, action, details, timestamp FROM activity WHERE user_id = ?1 ORDER BY id DESC LIMIT ?2"
        ).map_err(|e| format!("{e}"))?;
        let rows = stmt.query_map(params![user_id, limit as i64], |row| {
            Ok(ActivityEntry {
                id: row.get(0)?,
                wallet_address: row.get(1)?,
                action: row.get(2)?,
                details: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                timestamp: row.get(4)?,
            })
        }).map_err(|e| format!("{e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("{e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> UserDb {
        UserDb::open(":memory:").unwrap()
    }

    #[test]
    fn test_create_user() {
        let db = temp_db();
        let id = db.create_user("test@example.com", "$hash", "Test User", "123456").unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_duplicate_email_fails() {
        let db = temp_db();
        db.create_user("dup@example.com", "$hash", "User1", "111111").unwrap();
        let result = db.create_user("dup@example.com", "$hash2", "User2", "222222");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already registered"));
    }

    #[test]
    fn test_get_user_by_email() {
        let db = temp_db();
        db.create_user("find@example.com", "$hash", "Findme", "999999").unwrap();
        let user = db.get_user_by_email("find@example.com").unwrap();
        assert!(user.is_some());
        let (id, hash, name, verified, _code) = user.unwrap();
        assert!(id > 0);
        assert_eq!(hash, "$hash");
        assert_eq!(name, "Findme");
        assert!(!verified);
    }

    #[test]
    fn test_verify_email() {
        let db = temp_db();
        db.create_user("verify@example.com", "$h", "V", "654321").unwrap();
        assert!(!db.verify_email("verify@example.com", "wrong").unwrap());
        assert!(db.verify_email("verify@example.com", "654321").unwrap());
        // Already verified — returns false
        assert!(!db.verify_email("verify@example.com", "654321").unwrap());
    }

    #[test]
    fn test_create_and_list_wallets() {
        let db = temp_db();
        let uid = db.create_user("wallet@example.com", "$h", "W", "000000").unwrap();
        db.create_wallet(uid, "Main", "0xabc123", "pub123", "enc_priv123").unwrap();
        db.create_wallet(uid, "Trading", "0xdef456", "pub456", "enc_priv456").unwrap();
        let wallets = db.list_wallets(uid).unwrap();
        assert_eq!(wallets.len(), 2);
        assert_eq!(wallets[0].name, "Main");
        assert_eq!(wallets[1].address, "0xdef456");
    }

    #[test]
    fn test_wallet_owner() {
        let db = temp_db();
        let uid = db.create_user("own@example.com", "$h", "O", "000000").unwrap();
        db.create_wallet(uid, "Mine", "0xowner", "pub", "enc").unwrap();
        assert_eq!(db.get_wallet_owner("0xowner").unwrap(), Some(uid));
        assert_eq!(db.get_wallet_owner("0xother").unwrap(), None);
    }

    #[test]
    fn test_activity_log() {
        let db = temp_db();
        let uid = db.create_user("act@example.com", "$h", "A", "000000").unwrap();
        db.log_activity(uid, "0xaddr", "transfer", "Sent 500 EVAP").unwrap();
        db.log_activity(uid, "0xaddr", "faucet", "Claimed 10000 EVAP").unwrap();
        let activity = db.get_activity(uid, 10).unwrap();
        assert_eq!(activity.len(), 2);
        assert_eq!(activity[0].action, "faucet"); // most recent first
    }

    #[test]
    fn test_password_hash_verification() {
        let hash = bcrypt::hash("mypassword123", 4).unwrap();
        assert!(bcrypt::verify("mypassword123", &hash).unwrap());
        assert!(!bcrypt::verify("wrongpassword", &hash).unwrap());
    }
}
