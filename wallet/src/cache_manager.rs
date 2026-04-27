use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CacheManagerError {
    #[error("cache not found: {0}")]
    CacheNotFound(String),
    #[error("duplicate cache: {0}")]
    DuplicateCache(String),
    #[error("key not found: {0}")]
    KeyNotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvictionPolicy {
    #[default]
    Lru,
    Lfu,
    Fifo,
    Ttl,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum CacheLayer {
    #[default]
    Memory,
    Disk,
    Remote,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub value: String,
    pub layer: CacheLayer,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub access_count: u64,
    pub last_accessed: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig2 {
    pub id: String,
    pub name: String,
    pub layer: CacheLayer,
    pub max_entries: usize,
    pub eviction: EvictionPolicy,
    pub default_ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheStats2 {
    pub total_caches: usize,
    pub total_entries: usize,
    pub total_hits: u64,
    pub total_misses: u64,
    pub hit_rate: f64,
    pub total_evictions: u64,
    pub total_size_bytes: u64,
    pub by_layer: HashMap<String, usize>,
}

// ---------------------------------------------------------------------------
// CacheManager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CacheManager {
    pub caches: HashMap<String, CacheConfig2>,
    pub entries: HashMap<String, HashMap<String, CacheEntry>>,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl CacheManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_cache(&mut self, config: CacheConfig2) -> Result<(), CacheManagerError> {
        if self.caches.contains_key(&config.id) {
            return Err(CacheManagerError::DuplicateCache(config.id.clone()));
        }
        let id = config.id.clone();
        self.caches.insert(id.clone(), config);
        self.entries.insert(id, HashMap::new());
        Ok(())
    }

    pub fn remove_cache(&mut self, id: &str) -> Result<CacheConfig2, CacheManagerError> {
        let config = self
            .caches
            .remove(id)
            .ok_or_else(|| CacheManagerError::CacheNotFound(id.to_string()))?;
        self.entries.remove(id);
        Ok(config)
    }

    pub fn put(
        &mut self,
        cache_id: &str,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<(), CacheManagerError> {
        let config = self
            .caches
            .get(cache_id)
            .ok_or_else(|| CacheManagerError::CacheNotFound(cache_id.to_string()))?;
        let layer = config.layer.clone();
        let max_entries = config.max_entries;
        let eviction = config.eviction.clone();
        let default_ttl = config.default_ttl_seconds;

        let cache = self
            .entries
            .get_mut(cache_id)
            .ok_or_else(|| CacheManagerError::CacheNotFound(cache_id.to_string()))?;

        // Evict if at capacity (and the key is not already present)
        if cache.len() >= max_entries && !cache.contains_key(key) {
            Self::do_evict(cache, &eviction);
            self.evictions += 1;
        }

        let now = Utc::now().to_rfc3339();
        let effective_ttl = ttl_seconds.or(default_ttl);
        let expires_at = effective_ttl.map(|secs| {
            (Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339()
        });

        let entry = CacheEntry {
            key: key.to_string(),
            value: value.to_string(),
            layer,
            created_at: now,
            expires_at,
            access_count: 0,
            last_accessed: None,
            size_bytes: value.len() as u64,
        };

        let cache = self.entries.get_mut(cache_id).unwrap();
        cache.insert(key.to_string(), entry);
        Ok(())
    }

    pub fn get(
        &mut self,
        cache_id: &str,
        key: &str,
    ) -> Result<Option<String>, CacheManagerError> {
        if !self.caches.contains_key(cache_id) {
            return Err(CacheManagerError::CacheNotFound(cache_id.to_string()));
        }

        let cache = self
            .entries
            .get_mut(cache_id)
            .ok_or_else(|| CacheManagerError::CacheNotFound(cache_id.to_string()))?;

        let entry = match cache.get(key) {
            Some(e) => e,
            None => {
                self.misses += 1;
                return Ok(None);
            }
        };

        // Check TTL expiry
        if let Some(ref exp) = entry.expires_at {
            if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
                if Utc::now() >= exp_time {
                    cache.remove(key);
                    self.misses += 1;
                    return Ok(None);
                }
            }
        }

        // Update access metadata
        let entry = cache.get_mut(key).unwrap();
        entry.access_count += 1;
        entry.last_accessed = Some(Utc::now().to_rfc3339());
        let value = entry.value.clone();

        self.hits += 1;
        Ok(Some(value))
    }

    pub fn invalidate(&mut self, cache_id: &str, key: &str) -> Result<(), CacheManagerError> {
        let cache = self
            .entries
            .get_mut(cache_id)
            .ok_or_else(|| CacheManagerError::CacheNotFound(cache_id.to_string()))?;
        cache
            .remove(key)
            .ok_or_else(|| CacheManagerError::KeyNotFound(key.to_string()))?;
        Ok(())
    }

    pub fn invalidate_all(&mut self, cache_id: &str) -> Result<(), CacheManagerError> {
        let cache = self
            .entries
            .get_mut(cache_id)
            .ok_or_else(|| CacheManagerError::CacheNotFound(cache_id.to_string()))?;
        cache.clear();
        Ok(())
    }

    pub fn evict_expired(&mut self) {
        let now = Utc::now();
        for (_cache_id, cache) in self.entries.iter_mut() {
            cache.retain(|_key, entry| {
                if let Some(ref exp) = entry.expires_at {
                    if let Ok(exp_time) = chrono::DateTime::parse_from_rfc3339(exp) {
                        return now < exp_time;
                    }
                }
                true
            });
        }
    }

    pub fn entries_in_cache(
        &self,
        cache_id: &str,
    ) -> Result<Vec<&CacheEntry>, CacheManagerError> {
        let cache = self
            .entries
            .get(cache_id)
            .ok_or_else(|| CacheManagerError::CacheNotFound(cache_id.to_string()))?;
        Ok(cache.values().collect())
    }

    pub fn cache_size(&self, cache_id: &str) -> Result<usize, CacheManagerError> {
        let cache = self
            .entries
            .get(cache_id)
            .ok_or_else(|| CacheManagerError::CacheNotFound(cache_id.to_string()))?;
        Ok(cache.len())
    }

    pub fn total_size_bytes(&self) -> u64 {
        self.entries
            .values()
            .flat_map(|c| c.values())
            .map(|e| e.size_bytes)
            .sum()
    }

    pub fn stats(&self) -> CacheStats2 {
        let total_entries: usize = self.entries.values().map(|c| c.len()).sum();
        let total_requests = self.hits + self.misses;
        let hit_rate = if total_requests > 0 {
            self.hits as f64 / total_requests as f64
        } else {
            0.0
        };

        let mut by_layer: HashMap<String, usize> = HashMap::new();
        for (cache_id, cache) in &self.entries {
            if let Some(config) = self.caches.get(cache_id) {
                let layer_name = format!("{:?}", config.layer);
                *by_layer.entry(layer_name).or_insert(0) += cache.len();
            }
        }

        CacheStats2 {
            total_caches: self.caches.len(),
            total_entries,
            total_hits: self.hits,
            total_misses: self.misses,
            hit_rate,
            total_evictions: self.evictions,
            total_size_bytes: self.total_size_bytes(),
            by_layer,
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), CacheManagerError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, CacheManagerError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn do_evict(
        cache: &mut HashMap<String, CacheEntry>,
        policy: &EvictionPolicy,
    ) {
        if cache.is_empty() {
            return;
        }

        let victim_key = match policy {
            EvictionPolicy::Lru => {
                // Remove entry with oldest last_accessed (None treated as oldest)
                cache
                    .iter()
                    .min_by(|a, b| {
                        let la = a.1.last_accessed.as_deref().unwrap_or("");
                        let lb = b.1.last_accessed.as_deref().unwrap_or("");
                        la.cmp(lb)
                    })
                    .map(|(k, _)| k.clone())
            }
            EvictionPolicy::Lfu => {
                cache
                    .iter()
                    .min_by_key(|(_, e)| e.access_count)
                    .map(|(k, _)| k.clone())
            }
            EvictionPolicy::Fifo => {
                cache
                    .iter()
                    .min_by(|a, b| a.1.created_at.cmp(&b.1.created_at))
                    .map(|(k, _)| k.clone())
            }
            EvictionPolicy::Ttl => {
                let now = Utc::now().to_rfc3339();
                // First try to find an expired entry
                let expired = cache.iter().find(|(_, e)| {
                    e.expires_at
                        .as_deref()
                        .map(|exp| exp <= now.as_str())
                        .unwrap_or(false)
                });
                if let Some((k, _)) = expired {
                    Some(k.clone())
                } else {
                    // Fallback: oldest created_at
                    cache
                        .iter()
                        .min_by(|a, b| a.1.created_at.cmp(&b.1.created_at))
                        .map(|(k, _)| k.clone())
                }
            }
        };

        if let Some(key) = victim_key {
            cache.remove(&key);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_cache_test_{}_{}.json",
            std::process::id(),
            name
        ))
    }

    fn memory_config(id: &str, max: usize, eviction: EvictionPolicy) -> CacheConfig2 {
        CacheConfig2 {
            id: id.to_string(),
            name: format!("Cache {}", id),
            layer: CacheLayer::Memory,
            max_entries: max,
            eviction,
            default_ttl_seconds: None,
        }
    }

    #[test]
    fn test_create_cache() {
        let mut mgr = CacheManager::new();
        let cfg = memory_config("c1", 100, EvictionPolicy::Lru);
        mgr.create_cache(cfg).unwrap();
        assert_eq!(mgr.caches.len(), 1);
    }

    #[test]
    fn test_create_duplicate_cache() {
        let mut mgr = CacheManager::new();
        let cfg = memory_config("c1", 100, EvictionPolicy::Lru);
        mgr.create_cache(cfg.clone()).unwrap();
        let err = mgr.create_cache(cfg).unwrap_err();
        assert!(matches!(err, CacheManagerError::DuplicateCache(_)));
    }

    #[test]
    fn test_remove_cache() {
        let mut mgr = CacheManager::new();
        let cfg = memory_config("c1", 100, EvictionPolicy::Lru);
        mgr.create_cache(cfg).unwrap();
        let removed = mgr.remove_cache("c1").unwrap();
        assert_eq!(removed.id, "c1");
        assert!(mgr.caches.is_empty());
    }

    #[test]
    fn test_remove_cache_not_found() {
        let mut mgr = CacheManager::new();
        let err = mgr.remove_cache("nope").unwrap_err();
        assert!(matches!(err, CacheManagerError::CacheNotFound(_)));
    }

    #[test]
    fn test_put_and_get() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "hello", None).unwrap();
        let val = mgr.get("c1", "k1").unwrap();
        assert_eq!(val, Some("hello".to_string()));
    }

    #[test]
    fn test_get_missing_key() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        let val = mgr.get("c1", "nonexistent").unwrap();
        assert!(val.is_none());
        assert_eq!(mgr.misses, 1);
    }

    #[test]
    fn test_ttl_expiry() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        // TTL of 0 seconds means it's already expired
        mgr.put("c1", "k1", "value", Some(0)).unwrap();
        // The entry was just created with expires_at = now, so get should see it as expired
        std::thread::sleep(std::time::Duration::from_millis(10));
        let val = mgr.get("c1", "k1").unwrap();
        assert!(val.is_none());
        assert_eq!(mgr.misses, 1);
    }

    #[test]
    fn test_invalidate_key() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "val", None).unwrap();
        mgr.invalidate("c1", "k1").unwrap();
        let val = mgr.get("c1", "k1").unwrap();
        assert!(val.is_none());
    }

    #[test]
    fn test_invalidate_key_not_found() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        let err = mgr.invalidate("c1", "nope").unwrap_err();
        assert!(matches!(err, CacheManagerError::KeyNotFound(_)));
    }

    #[test]
    fn test_invalidate_all() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "v1", None).unwrap();
        mgr.put("c1", "k2", "v2", None).unwrap();
        mgr.invalidate_all("c1").unwrap();
        assert_eq!(mgr.cache_size("c1").unwrap(), 0);
    }

    #[test]
    fn test_lru_eviction() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 2, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "v1", None).unwrap();
        mgr.put("c1", "k2", "v2", None).unwrap();
        // Access k1 so k2 becomes least recently used
        mgr.get("c1", "k1").unwrap();
        // Insert k3 => should evict k2 (never accessed, last_accessed is None)
        mgr.put("c1", "k3", "v3", None).unwrap();
        assert!(mgr.get("c1", "k2").unwrap().is_none());
        assert!(mgr.get("c1", "k1").unwrap().is_some());
    }

    #[test]
    fn test_lfu_eviction() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 2, EvictionPolicy::Lfu))
            .unwrap();
        mgr.put("c1", "k1", "v1", None).unwrap();
        mgr.put("c1", "k2", "v2", None).unwrap();
        // Access k1 multiple times to increase its access_count
        mgr.get("c1", "k1").unwrap();
        mgr.get("c1", "k1").unwrap();
        // k2 has access_count=0, k1 has access_count=2 => k2 should be evicted
        mgr.put("c1", "k3", "v3", None).unwrap();
        assert!(mgr.get("c1", "k2").unwrap().is_none());
        assert!(mgr.get("c1", "k1").unwrap().is_some());
    }

    #[test]
    fn test_fifo_eviction() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 2, EvictionPolicy::Fifo))
            .unwrap();
        mgr.put("c1", "k1", "v1", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        mgr.put("c1", "k2", "v2", None).unwrap();
        // k1 was created first => should be evicted
        mgr.put("c1", "k3", "v3", None).unwrap();
        assert!(mgr.get("c1", "k1").unwrap().is_none());
        assert!(mgr.get("c1", "k2").unwrap().is_some());
    }

    #[test]
    fn test_hit_miss_tracking() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "v1", None).unwrap();
        mgr.get("c1", "k1").unwrap(); // hit
        mgr.get("c1", "k1").unwrap(); // hit
        mgr.get("c1", "missing").unwrap(); // miss
        assert_eq!(mgr.hits, 2);
        assert_eq!(mgr.misses, 1);
    }

    #[test]
    fn test_cache_size() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "v1", None).unwrap();
        mgr.put("c1", "k2", "v2", None).unwrap();
        assert_eq!(mgr.cache_size("c1").unwrap(), 2);
    }

    #[test]
    fn test_total_size_bytes() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "hello", None).unwrap(); // 5 bytes
        mgr.put("c1", "k2", "world!", None).unwrap(); // 6 bytes
        assert_eq!(mgr.total_size_bytes(), 11);
    }

    #[test]
    fn test_stats() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "v1", None).unwrap();
        mgr.get("c1", "k1").unwrap();
        mgr.get("c1", "miss").unwrap();
        let s = mgr.stats();
        assert_eq!(s.total_caches, 1);
        assert_eq!(s.total_entries, 1);
        assert_eq!(s.total_hits, 1);
        assert_eq!(s.total_misses, 1);
        assert!((s.hit_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_save_and_load() {
        let path = test_path("save_load");
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "persisted", None).unwrap();
        mgr.save(&path).unwrap();

        let loaded = CacheManager::load(&path).unwrap();
        assert_eq!(loaded.caches.len(), 1);
        assert!(loaded.entries.get("c1").unwrap().contains_key("k1"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("load_or_default_missing");
        let _ = std::fs::remove_file(&path);
        let mgr = CacheManager::load_or_default(&path);
        assert!(mgr.caches.is_empty());
    }

    #[test]
    fn test_evict_expired() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "val", Some(0)).unwrap();
        mgr.put("c1", "k2", "val2", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        mgr.evict_expired();
        assert_eq!(mgr.cache_size("c1").unwrap(), 1);
        assert!(mgr.entries.get("c1").unwrap().contains_key("k2"));
    }

    #[test]
    fn test_put_cache_not_found() {
        let mut mgr = CacheManager::new();
        let err = mgr.put("nope", "k", "v", None).unwrap_err();
        assert!(matches!(err, CacheManagerError::CacheNotFound(_)));
    }

    #[test]
    fn test_entries_in_cache() {
        let mut mgr = CacheManager::new();
        mgr.create_cache(memory_config("c1", 10, EvictionPolicy::Lru))
            .unwrap();
        mgr.put("c1", "k1", "v1", None).unwrap();
        mgr.put("c1", "k2", "v2", None).unwrap();
        let entries = mgr.entries_in_cache("c1").unwrap();
        assert_eq!(entries.len(), 2);
    }
}
