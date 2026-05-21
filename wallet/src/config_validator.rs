use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ConfigValidatorError {
    #[error("schema not found: {0}")]
    SchemaNotFound(String),
    #[error("duplicate schema: {0}")]
    DuplicateSchema(String),
    #[error("validation failed: {0}")]
    ValidationFailed(String),
    #[error("migration failed: {0}")]
    MigrationFailed(String),
    #[error("rollback failed: {0}")]
    RollbackFailed(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FieldType {
    String,
    Integer,
    Float,
    Boolean,
    Array,
    Object,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ValidationSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum MigrationStatus2 {
    #[default]
    Pending,
    Applied,
    RolledBack,
    Failed,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    pub name: String,
    pub field_type: FieldType,
    pub required: bool,
    pub default_value: Option<String>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSchema {
    pub id: String,
    pub version: u32,
    pub name: String,
    pub fields: Vec<FieldSchema>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
    pub severity: ValidationSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult2 {
    pub schema_id: String,
    pub valid: bool,
    pub errors: Vec<ValidationIssue>,
    pub warnings: Vec<ValidationIssue>,
    pub validated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationChange {
    pub field: String,
    pub action: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMigration {
    pub id: String,
    pub from_version: u32,
    pub to_version: u32,
    pub description: String,
    pub changes: Vec<MigrationChange>,
    pub status: MigrationStatus2,
    pub applied_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBackup2 {
    pub version: u32,
    pub data: HashMap<String, String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorStats {
    pub total_schemas: usize,
    pub total_validations: usize,
    pub total_migrations: usize,
    pub applied_migrations: usize,
    pub rolled_back: usize,
    pub failed_validations: usize,
    pub backups: usize,
}

// ---------------------------------------------------------------------------
// ConfigValidator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigValidator {
    pub schemas: HashMap<String, ConfigSchema>,
    pub migrations: Vec<ConfigMigration>,
    pub backups: Vec<ConfigBackup2>,
    pub validation_history: Vec<ValidationResult2>,
}

impl ConfigValidator {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Schema management --------------------------------------------------

    pub fn register_schema(&mut self, schema: ConfigSchema) -> Result<(), ConfigValidatorError> {
        if self.schemas.contains_key(&schema.id) {
            return Err(ConfigValidatorError::DuplicateSchema(schema.id.clone()));
        }
        self.schemas.insert(schema.id.clone(), schema);
        Ok(())
    }

    pub fn remove_schema(&mut self, id: &str) -> Result<ConfigSchema, ConfigValidatorError> {
        self.schemas
            .remove(id)
            .ok_or_else(|| ConfigValidatorError::SchemaNotFound(id.to_string()))
    }

    pub fn get_schema(&self, id: &str) -> Option<&ConfigSchema> {
        self.schemas.get(id)
    }

    pub fn schemas_by_version(&self, version: u32) -> Vec<&ConfigSchema> {
        self.schemas
            .values()
            .filter(|s| s.version == version)
            .collect()
    }

    // -- Validation ---------------------------------------------------------

    pub fn validate(
        &mut self,
        schema_id: &str,
        config: &HashMap<String, String>,
    ) -> Result<ValidationResult2, ConfigValidatorError> {
        let schema = self
            .schemas
            .get(schema_id)
            .ok_or_else(|| ConfigValidatorError::SchemaNotFound(schema_id.to_string()))?
            .clone();

        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        for field in &schema.fields {
            match config.get(&field.name) {
                None => {
                    if field.required {
                        errors.push(ValidationIssue {
                            field: field.name.clone(),
                            message: format!("required field '{}' is missing", field.name),
                            severity: ValidationSeverity::Error,
                        });
                    } else {
                        warnings.push(ValidationIssue {
                            field: field.name.clone(),
                            message: format!("optional field '{}' is missing", field.name),
                            severity: ValidationSeverity::Warning,
                        });
                    }
                }
                Some(value) => {
                    self.validate_field_type(field, value, &mut errors);
                    self.validate_field_bounds(field, value, &mut errors);
                }
            }
        }

        let valid = errors.is_empty();
        let result = ValidationResult2 {
            schema_id: schema_id.to_string(),
            valid,
            errors,
            warnings,
            validated_at: Utc::now().to_rfc3339(),
        };

        self.validation_history.push(result.clone());
        Ok(result)
    }

    fn validate_field_type(
        &self,
        field: &FieldSchema,
        value: &str,
        errors: &mut Vec<ValidationIssue>,
    ) {
        match field.field_type {
            FieldType::Integer => {
                if value.parse::<i64>().is_err() {
                    errors.push(ValidationIssue {
                        field: field.name.clone(),
                        message: format!("'{}' is not a valid integer", value),
                        severity: ValidationSeverity::Error,
                    });
                }
            }
            FieldType::Float => {
                if value.parse::<f64>().is_err() {
                    errors.push(ValidationIssue {
                        field: field.name.clone(),
                        message: format!("'{}' is not a valid float", value),
                        severity: ValidationSeverity::Error,
                    });
                }
            }
            FieldType::Boolean => {
                if value != "true" && value != "false" {
                    errors.push(ValidationIssue {
                        field: field.name.clone(),
                        message: format!(
                            "'{}' is not a valid boolean (expected true/false)",
                            value
                        ),
                        severity: ValidationSeverity::Error,
                    });
                }
            }
            _ => {}
        }
    }

    fn validate_field_bounds(
        &self,
        field: &FieldSchema,
        value: &str,
        errors: &mut Vec<ValidationIssue>,
    ) {
        let numeric: Option<f64> = match field.field_type {
            FieldType::Integer => value.parse::<i64>().ok().map(|v| v as f64),
            FieldType::Float => value.parse::<f64>().ok(),
            _ => None,
        };

        if let Some(num) = numeric {
            if let Some(min) = field.min_value {
                if num < min {
                    errors.push(ValidationIssue {
                        field: field.name.clone(),
                        message: format!("value {} is below minimum {}", num, min),
                        severity: ValidationSeverity::Error,
                    });
                }
            }
            if let Some(max) = field.max_value {
                if num > max {
                    errors.push(ValidationIssue {
                        field: field.name.clone(),
                        message: format!("value {} exceeds maximum {}", num, max),
                        severity: ValidationSeverity::Error,
                    });
                }
            }
        }
    }

    // -- Migrations ---------------------------------------------------------

    pub fn register_migration(
        &mut self,
        migration: ConfigMigration,
    ) -> Result<(), ConfigValidatorError> {
        if self.migrations.iter().any(|m| m.id == migration.id) {
            return Err(ConfigValidatorError::DuplicateSchema(format!(
                "migration '{}' already registered",
                migration.id
            )));
        }
        self.migrations.push(migration);
        Ok(())
    }

    pub fn apply_migration(
        &mut self,
        migration_id: &str,
        config: &mut HashMap<String, String>,
    ) -> Result<(), ConfigValidatorError> {
        let idx = self
            .migrations
            .iter()
            .position(|m| m.id == migration_id)
            .ok_or_else(|| {
                ConfigValidatorError::MigrationFailed(format!(
                    "migration '{}' not found",
                    migration_id
                ))
            })?;

        // Create backup before applying
        let version = self.migrations[idx].from_version;
        self.create_backup(version, config);

        // Apply changes
        let changes = self.migrations[idx].changes.clone();
        for change in &changes {
            match change.action.as_str() {
                "add" => {
                    if let Some(ref val) = change.new_value {
                        config.insert(change.field.clone(), val.clone());
                    }
                }
                "remove" => {
                    config.remove(&change.field);
                }
                "rename" => {
                    if let Some(val) = config.remove(&change.field) {
                        if let Some(ref new_name) = change.new_value {
                            config.insert(new_name.clone(), val);
                        }
                    }
                }
                "update" => {
                    if let Some(ref val) = change.new_value {
                        config.insert(change.field.clone(), val.clone());
                    }
                }
                _ => {}
            }
        }

        self.migrations[idx].status = MigrationStatus2::Applied;
        self.migrations[idx].applied_at = Some(Utc::now().to_rfc3339());
        Ok(())
    }

    pub fn rollback_migration(
        &mut self,
        migration_id: &str,
        config: &mut HashMap<String, String>,
    ) -> Result<(), ConfigValidatorError> {
        let idx = self
            .migrations
            .iter()
            .position(|m| m.id == migration_id)
            .ok_or_else(|| {
                ConfigValidatorError::RollbackFailed(format!(
                    "migration '{}' not found",
                    migration_id
                ))
            })?;

        let backup = self
            .backups
            .last()
            .ok_or_else(|| {
                ConfigValidatorError::RollbackFailed("no backup available for rollback".to_string())
            })?
            .clone();

        config.clear();
        for (k, v) in &backup.data {
            config.insert(k.clone(), v.clone());
        }

        self.migrations[idx].status = MigrationStatus2::RolledBack;
        Ok(())
    }

    // -- Backups ------------------------------------------------------------

    pub fn create_backup(&mut self, version: u32, config: &HashMap<String, String>) {
        self.backups.push(ConfigBackup2 {
            version,
            data: config.clone(),
            created_at: Utc::now().to_rfc3339(),
        });
    }

    pub fn latest_backup(&self) -> Option<&ConfigBackup2> {
        self.backups.last()
    }

    // -- Queries ------------------------------------------------------------

    pub fn pending_migrations(&self) -> Vec<&ConfigMigration> {
        self.migrations
            .iter()
            .filter(|m| m.status == MigrationStatus2::Pending)
            .collect()
    }

    pub fn applied_migrations(&self) -> Vec<&ConfigMigration> {
        self.migrations
            .iter()
            .filter(|m| m.status == MigrationStatus2::Applied)
            .collect()
    }

    pub fn validation_history_for(&self, schema_id: &str) -> Vec<&ValidationResult2> {
        self.validation_history
            .iter()
            .filter(|r| r.schema_id == schema_id)
            .collect()
    }

    pub fn stats(&self) -> ValidatorStats {
        let failed_validations = self.validation_history.iter().filter(|r| !r.valid).count();

        ValidatorStats {
            total_schemas: self.schemas.len(),
            total_validations: self.validation_history.len(),
            total_migrations: self.migrations.len(),
            applied_migrations: self
                .migrations
                .iter()
                .filter(|m| m.status == MigrationStatus2::Applied)
                .count(),
            rolled_back: self
                .migrations
                .iter()
                .filter(|m| m.status == MigrationStatus2::RolledBack)
                .count(),
            failed_validations,
            backups: self.backups.len(),
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), ConfigValidatorError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ConfigValidatorError> {
        let data = std::fs::read_to_string(path)?;
        let validator: Self = serde_json::from_str(&data)?;
        Ok(validator)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_schema() -> ConfigSchema {
        ConfigSchema {
            id: "wallet_config".to_string(),
            version: 1,
            name: "Wallet Configuration".to_string(),
            fields: vec![
                FieldSchema {
                    name: "rpc_url".to_string(),
                    field_type: FieldType::String,
                    required: true,
                    default_value: None,
                    min_value: None,
                    max_value: None,
                    description: "RPC endpoint URL".to_string(),
                },
                FieldSchema {
                    name: "max_retries".to_string(),
                    field_type: FieldType::Integer,
                    required: true,
                    default_value: Some("3".to_string()),
                    min_value: Some(1.0),
                    max_value: Some(10.0),
                    description: "Maximum retry attempts".to_string(),
                },
                FieldSchema {
                    name: "gas_multiplier".to_string(),
                    field_type: FieldType::Float,
                    required: false,
                    default_value: Some("1.5".to_string()),
                    min_value: Some(1.0),
                    max_value: Some(5.0),
                    description: "Gas price multiplier".to_string(),
                },
                FieldSchema {
                    name: "debug_mode".to_string(),
                    field_type: FieldType::Boolean,
                    required: true,
                    default_value: Some("false".to_string()),
                    min_value: None,
                    max_value: None,
                    description: "Enable debug mode".to_string(),
                },
            ],
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn valid_config() -> HashMap<String, String> {
        let mut config = HashMap::new();
        config.insert(
            "rpc_url".to_string(),
            "https://rpc.evaporchain.io".to_string(),
        );
        config.insert("max_retries".to_string(), "5".to_string());
        config.insert("gas_multiplier".to_string(), "2.0".to_string());
        config.insert("debug_mode".to_string(), "true".to_string());
        config
    }

    fn sample_migration() -> ConfigMigration {
        ConfigMigration {
            id: "mig_001".to_string(),
            from_version: 1,
            to_version: 2,
            description: "Add timeout field".to_string(),
            changes: vec![
                MigrationChange {
                    field: "timeout_ms".to_string(),
                    action: "add".to_string(),
                    old_value: None,
                    new_value: Some("30000".to_string()),
                },
                MigrationChange {
                    field: "legacy_flag".to_string(),
                    action: "remove".to_string(),
                    old_value: Some("yes".to_string()),
                    new_value: None,
                },
            ],
            status: MigrationStatus2::Pending,
            applied_at: None,
        }
    }

    fn temp_path(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_test_{}_{}.json",
            std::process::id(),
            suffix
        ))
    }

    // 1
    #[test]
    fn test_register_schema() {
        let mut v = ConfigValidator::new();
        let schema = sample_schema();
        assert!(v.register_schema(schema).is_ok());
        assert_eq!(v.schemas.len(), 1);
    }

    // 2
    #[test]
    fn test_register_duplicate_schema() {
        let mut v = ConfigValidator::new();
        let schema = sample_schema();
        v.register_schema(schema.clone()).unwrap();
        let result = v.register_schema(schema);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigValidatorError::DuplicateSchema(id) => assert_eq!(id, "wallet_config"),
            other => panic!("expected DuplicateSchema, got {:?}", other),
        }
    }

    // 3
    #[test]
    fn test_remove_schema() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let removed = v.remove_schema("wallet_config").unwrap();
        assert_eq!(removed.id, "wallet_config");
        assert!(v.schemas.is_empty());
    }

    // 4
    #[test]
    fn test_remove_schema_not_found() {
        let mut v = ConfigValidator::new();
        let result = v.remove_schema("nonexistent");
        assert!(result.is_err());
    }

    // 5
    #[test]
    fn test_validate_valid_config() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let config = valid_config();
        let result = v.validate("wallet_config", &config).unwrap();
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    // 6
    #[test]
    fn test_validate_missing_required_field() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let mut config = valid_config();
        config.remove("rpc_url");
        let result = v.validate("wallet_config", &config).unwrap();
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.field == "rpc_url"));
    }

    // 7
    #[test]
    fn test_validate_wrong_integer_type() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let mut config = valid_config();
        config.insert("max_retries".to_string(), "not_a_number".to_string());
        let result = v.validate("wallet_config", &config).unwrap();
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.field == "max_retries"));
    }

    // 8
    #[test]
    fn test_validate_wrong_boolean_type() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let mut config = valid_config();
        config.insert("debug_mode".to_string(), "yes".to_string());
        let result = v.validate("wallet_config", &config).unwrap();
        assert!(!result.valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.field == "debug_mode" && e.message.contains("boolean")));
    }

    // 9
    #[test]
    fn test_validate_min_bound() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let mut config = valid_config();
        config.insert("max_retries".to_string(), "0".to_string());
        let result = v.validate("wallet_config", &config).unwrap();
        assert!(!result.valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.field == "max_retries" && e.message.contains("below minimum")));
    }

    // 10
    #[test]
    fn test_validate_max_bound() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let mut config = valid_config();
        config.insert("gas_multiplier".to_string(), "10.0".to_string());
        let result = v.validate("wallet_config", &config).unwrap();
        assert!(!result.valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.field == "gas_multiplier" && e.message.contains("exceeds maximum")));
    }

    // 11
    #[test]
    fn test_validate_schema_not_found() {
        let mut v = ConfigValidator::new();
        let config = HashMap::new();
        let result = v.validate("nonexistent", &config);
        assert!(result.is_err());
    }

    // 12
    #[test]
    fn test_validate_with_warnings() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let mut config = valid_config();
        // Remove optional field to trigger a warning
        config.remove("gas_multiplier");
        let result = v.validate("wallet_config", &config).unwrap();
        assert!(result.valid); // still valid since field is optional
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.field == "gas_multiplier"));
    }

    // 13
    #[test]
    fn test_apply_migration() {
        let mut v = ConfigValidator::new();
        v.register_migration(sample_migration()).unwrap();
        let mut config = HashMap::new();
        config.insert("legacy_flag".to_string(), "yes".to_string());

        v.apply_migration("mig_001", &mut config).unwrap();
        assert_eq!(config.get("timeout_ms").unwrap(), "30000");
        assert!(!config.contains_key("legacy_flag"));
        assert_eq!(v.migrations[0].status, MigrationStatus2::Applied);
    }

    // 14
    #[test]
    fn test_rollback_migration() {
        let mut v = ConfigValidator::new();
        v.register_migration(sample_migration()).unwrap();
        let mut config = HashMap::new();
        config.insert("legacy_flag".to_string(), "yes".to_string());

        v.apply_migration("mig_001", &mut config).unwrap();
        assert!(config.contains_key("timeout_ms"));

        v.rollback_migration("mig_001", &mut config).unwrap();
        assert!(config.contains_key("legacy_flag"));
        assert!(!config.contains_key("timeout_ms"));
        assert_eq!(v.migrations[0].status, MigrationStatus2::RolledBack);
    }

    // 15
    #[test]
    fn test_backup_creation() {
        let mut v = ConfigValidator::new();
        let config = valid_config();
        v.create_backup(1, &config);
        assert_eq!(v.backups.len(), 1);
        assert_eq!(v.backups[0].version, 1);
    }

    // 16
    #[test]
    fn test_latest_backup() {
        let mut v = ConfigValidator::new();
        assert!(v.latest_backup().is_none());

        let config1 = HashMap::new();
        v.create_backup(1, &config1);

        let mut config2 = HashMap::new();
        config2.insert("key".to_string(), "val".to_string());
        v.create_backup(2, &config2);

        let latest = v.latest_backup().unwrap();
        assert_eq!(latest.version, 2);
        assert!(latest.data.contains_key("key"));
    }

    // 17
    #[test]
    fn test_pending_migrations() {
        let mut v = ConfigValidator::new();
        v.register_migration(sample_migration()).unwrap();

        let pending = v.pending_migrations();
        assert_eq!(pending.len(), 1);

        let mut config = HashMap::new();
        v.apply_migration("mig_001", &mut config).unwrap();
        assert!(v.pending_migrations().is_empty());
    }

    // 18
    #[test]
    fn test_applied_migrations() {
        let mut v = ConfigValidator::new();
        v.register_migration(sample_migration()).unwrap();
        assert!(v.applied_migrations().is_empty());

        let mut config = HashMap::new();
        v.apply_migration("mig_001", &mut config).unwrap();
        assert_eq!(v.applied_migrations().len(), 1);
    }

    // 19
    #[test]
    fn test_validation_history() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let config = valid_config();
        v.validate("wallet_config", &config).unwrap();
        v.validate("wallet_config", &config).unwrap();

        let history = v.validation_history_for("wallet_config");
        assert_eq!(history.len(), 2);
        assert!(v.validation_history_for("other_schema").is_empty());
    }

    // 20
    #[test]
    fn test_schemas_by_version() {
        let mut v = ConfigValidator::new();
        let mut s1 = sample_schema();
        s1.id = "s1".to_string();
        s1.version = 1;
        let mut s2 = sample_schema();
        s2.id = "s2".to_string();
        s2.version = 2;
        let mut s3 = sample_schema();
        s3.id = "s3".to_string();
        s3.version = 1;

        v.register_schema(s1).unwrap();
        v.register_schema(s2).unwrap();
        v.register_schema(s3).unwrap();

        assert_eq!(v.schemas_by_version(1).len(), 2);
        assert_eq!(v.schemas_by_version(2).len(), 1);
        assert_eq!(v.schemas_by_version(99).len(), 0);
    }

    // 21
    #[test]
    fn test_stats() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        v.register_migration(sample_migration()).unwrap();

        let config = valid_config();
        v.validate("wallet_config", &config).unwrap();

        // Trigger a failed validation
        let empty: HashMap<String, String> = HashMap::new();
        v.validate("wallet_config", &empty).unwrap();

        let mut mutable_config = HashMap::new();
        v.apply_migration("mig_001", &mut mutable_config).unwrap();

        let stats = v.stats();
        assert_eq!(stats.total_schemas, 1);
        assert_eq!(stats.total_validations, 2);
        assert_eq!(stats.total_migrations, 1);
        assert_eq!(stats.applied_migrations, 1);
        assert_eq!(stats.failed_validations, 1);
        assert_eq!(stats.backups, 1); // created during migration apply
    }

    // 22
    #[test]
    fn test_save_load_persistence() {
        let path = temp_path("persistence");

        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        v.register_migration(sample_migration()).unwrap();
        let config = valid_config();
        v.validate("wallet_config", &config).unwrap();

        v.save(&path).unwrap();

        let loaded = ConfigValidator::load(&path).unwrap();
        assert_eq!(loaded.schemas.len(), 1);
        assert_eq!(loaded.migrations.len(), 1);
        assert_eq!(loaded.validation_history.len(), 1);

        // Clean up
        let _ = std::fs::remove_file(&path);
    }

    // 23 (bonus)
    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("load_or_default_missing");
        let _ = std::fs::remove_file(&path); // ensure it does not exist
        let v = ConfigValidator::load_or_default(&path);
        assert!(v.schemas.is_empty());
        assert!(v.migrations.is_empty());
    }

    #[test]
    fn test_get_schema_covers_lines_169_171() {
        let mut v = ConfigValidator::new();
        assert!(v.get_schema("wallet_config").is_none());
        v.register_schema(sample_schema()).unwrap();
        let schema = v.get_schema("wallet_config").unwrap();
        assert_eq!(schema.id, "wallet_config");
    }

    #[test]
    fn test_validate_wrong_float_type_covers_lines_251_255() {
        let mut v = ConfigValidator::new();
        v.register_schema(sample_schema()).unwrap();
        let mut config = valid_config();
        config.insert("gas_multiplier".to_string(), "not_a_float".to_string());
        let result = v.validate("wallet_config", &config).unwrap();
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.field == "gas_multiplier" && e.message.contains("not a valid float")));
    }

    #[test]
    fn test_register_duplicate_migration_covers_lines_315_318() {
        let mut v = ConfigValidator::new();
        v.register_migration(sample_migration()).unwrap();
        let err = v.register_migration(sample_migration()).unwrap_err();
        assert!(matches!(err, ConfigValidatorError::DuplicateSchema(_)));
    }

    #[test]
    fn test_apply_migration_not_found_covers_lines_334_338() {
        let mut v = ConfigValidator::new();
        let mut config = HashMap::new();
        let err = v.apply_migration("nonexistent_mig", &mut config).unwrap_err();
        assert!(matches!(err, ConfigValidatorError::MigrationFailed(_)));
    }

    #[test]
    fn test_apply_migration_rename_action_covers_lines_356_361() {
        let mut v = ConfigValidator::new();
        let mig = ConfigMigration {
            id: "mig_rename".to_string(),
            from_version: 1,
            to_version: 2,
            description: "rename a field".to_string(),
            changes: vec![MigrationChange {
                field: "old_key".to_string(),
                action: "rename".to_string(),
                old_value: None,
                new_value: Some("new_key".to_string()),
            }],
            status: MigrationStatus2::Pending,
            applied_at: None,
        };
        v.register_migration(mig).unwrap();
        let mut config = HashMap::new();
        config.insert("old_key".to_string(), "myvalue".to_string());
        v.apply_migration("mig_rename", &mut config).unwrap();
        assert!(!config.contains_key("old_key"));
        assert_eq!(config.get("new_key").unwrap(), "myvalue");
    }

    #[test]
    fn test_apply_migration_update_action_covers_lines_363_366() {
        let mut v = ConfigValidator::new();
        let mig = ConfigMigration {
            id: "mig_update".to_string(),
            from_version: 1,
            to_version: 2,
            description: "update a field".to_string(),
            changes: vec![MigrationChange {
                field: "rpc_url".to_string(),
                action: "update".to_string(),
                old_value: None,
                new_value: Some("https://new-rpc.example.com".to_string()),
            }],
            status: MigrationStatus2::Pending,
            applied_at: None,
        };
        v.register_migration(mig).unwrap();
        let mut config = HashMap::new();
        config.insert("rpc_url".to_string(), "https://old-rpc.example.com".to_string());
        v.apply_migration("mig_update", &mut config).unwrap();
        assert_eq!(config.get("rpc_url").unwrap(), "https://new-rpc.example.com");
    }

    #[test]
    fn test_apply_migration_unknown_action_covers_line_368() {
        let mut v = ConfigValidator::new();
        let mig = ConfigMigration {
            id: "mig_unknown".to_string(),
            from_version: 1,
            to_version: 2,
            description: "unknown action".to_string(),
            changes: vec![MigrationChange {
                field: "some_field".to_string(),
                action: "noop".to_string(),
                old_value: None,
                new_value: None,
            }],
            status: MigrationStatus2::Pending,
            applied_at: None,
        };
        v.register_migration(mig).unwrap();
        let mut config = HashMap::new();
        v.apply_migration("mig_unknown", &mut config).unwrap();
    }

    #[test]
    fn test_rollback_migration_not_found_covers_lines_387_391() {
        let mut v = ConfigValidator::new();
        let mut config = HashMap::new();
        let err = v.rollback_migration("nonexistent_mig", &mut config).unwrap_err();
        assert!(matches!(err, ConfigValidatorError::RollbackFailed(_)));
    }

    #[test]
    fn test_rollback_migration_no_backup_covers_lines_397_398() {
        let mut v = ConfigValidator::new();
        v.register_migration(sample_migration()).unwrap();
        // Don't apply first, so no backup exists
        let mut config = HashMap::new();
        let err = v.rollback_migration("mig_001", &mut config).unwrap_err();
        assert!(matches!(err, ConfigValidatorError::RollbackFailed(_)));
    }
}
