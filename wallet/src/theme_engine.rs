use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ThemeEngineError {
    #[error("theme not found: {0}")]
    ThemeNotFound(String),
    #[error("duplicate theme: {0}")]
    DuplicateTheme(String),
    #[error("preset not found: {0}")]
    PresetNotFound(String),
    #[error("invalid color: {0}")]
    InvalidColor(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
    HighContrast,
    Custom(String),
}

impl std::fmt::Display for ColorScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Light => write!(f, "Light"),
            Self::Dark => write!(f, "Dark"),
            Self::HighContrast => write!(f, "HighContrast"),
            Self::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LayoutPreset {
    Compact,
    #[default]
    Standard,
    Detailed,
    Minimal,
}

// ---------------------------------------------------------------------------
// ThemeColors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeColors {
    pub primary: String,
    pub secondary: String,
    pub accent: String,
    pub background: String,
    pub text: String,
    pub error: String,
    pub warning: String,
    pub success: String,
    pub info: String,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self::default_light()
    }
}

impl ThemeColors {
    pub fn default_light() -> Self {
        Self {
            primary: "#1A73E8".into(),
            secondary: "#5F6368".into(),
            accent: "#FBBC04".into(),
            background: "#FFFFFF".into(),
            text: "#202124".into(),
            error: "#D93025".into(),
            warning: "#F9AB00".into(),
            success: "#1E8E3E".into(),
            info: "#1A73E8".into(),
        }
    }

    pub fn default_dark() -> Self {
        Self {
            primary: "#8AB4F8".into(),
            secondary: "#9AA0A6".into(),
            accent: "#FDD663".into(),
            background: "#202124".into(),
            text: "#E8EAED".into(),
            error: "#F28B82".into(),
            warning: "#FDD663".into(),
            success: "#81C995".into(),
            info: "#8AB4F8".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub description: String,
    pub scheme: ColorScheme,
    pub layout: LayoutPreset,
    pub colors: ThemeColors,
    pub custom_vars: HashMap<String, String>,
    pub created_at: String,
    pub is_builtin: bool,
}

// ---------------------------------------------------------------------------
// ThemeStats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeStats {
    pub total_themes: usize,
    pub builtin: usize,
    pub custom: usize,
    pub active_theme: Option<String>,
    pub schemes_used: HashMap<String, usize>,
}

// ---------------------------------------------------------------------------
// ThemeEngine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThemeEngine {
    pub themes: HashMap<String, Theme>,
    pub active_theme_id: Option<String>,
}

impl ThemeEngine {
    pub fn new() -> Self {
        Self::default()
    }

    // -- defaults ----------------------------------------------------------

    pub fn register_defaults(&mut self) {
        let now = Utc::now().to_rfc3339();

        let builtins = vec![
            Theme {
                id: "default_light".into(),
                name: "Default Light".into(),
                description: "Built-in light theme".into(),
                scheme: ColorScheme::Light,
                layout: LayoutPreset::Standard,
                colors: ThemeColors::default_light(),
                custom_vars: HashMap::new(),
                created_at: now.clone(),
                is_builtin: true,
            },
            Theme {
                id: "default_dark".into(),
                name: "Default Dark".into(),
                description: "Built-in dark theme".into(),
                scheme: ColorScheme::Dark,
                layout: LayoutPreset::Standard,
                colors: ThemeColors::default_dark(),
                custom_vars: HashMap::new(),
                created_at: now.clone(),
                is_builtin: true,
            },
            Theme {
                id: "high_contrast".into(),
                name: "High Contrast".into(),
                description: "Built-in high-contrast theme".into(),
                scheme: ColorScheme::HighContrast,
                layout: LayoutPreset::Detailed,
                colors: ThemeColors::default_dark(),
                custom_vars: HashMap::new(),
                created_at: now.clone(),
                is_builtin: true,
            },
            Theme {
                id: "minimal".into(),
                name: "Minimal".into(),
                description: "Built-in minimal theme".into(),
                scheme: ColorScheme::Light,
                layout: LayoutPreset::Minimal,
                colors: ThemeColors::default_light(),
                custom_vars: HashMap::new(),
                created_at: now,
                is_builtin: true,
            },
        ];

        for theme in builtins {
            self.themes.insert(theme.id.clone(), theme);
        }
    }

    // -- CRUD --------------------------------------------------------------

    pub fn add_theme(&mut self, theme: Theme) -> Result<(), ThemeEngineError> {
        if self.themes.contains_key(&theme.id) {
            return Err(ThemeEngineError::DuplicateTheme(theme.id.clone()));
        }
        self.themes.insert(theme.id.clone(), theme);
        Ok(())
    }

    pub fn remove_theme(&mut self, id: &str) -> Result<Theme, ThemeEngineError> {
        match self.themes.get(id) {
            None => Err(ThemeEngineError::ThemeNotFound(id.to_string())),
            Some(t) if t.is_builtin => Err(ThemeEngineError::ThemeNotFound(format!(
                "cannot remove builtin theme: {}",
                id
            ))),
            _ => {
                let theme = self.themes.remove(id).unwrap();
                if self.active_theme_id.as_deref() == Some(id) {
                    self.active_theme_id = None;
                }
                Ok(theme)
            }
        }
    }

    pub fn update_theme(&mut self, id: &str, colors: ThemeColors) -> Result<(), ThemeEngineError> {
        let theme = self
            .themes
            .get_mut(id)
            .ok_or_else(|| ThemeEngineError::ThemeNotFound(id.to_string()))?;
        theme.colors = colors;
        Ok(())
    }

    // -- active ------------------------------------------------------------

    pub fn set_active(&mut self, id: &str) -> Result<(), ThemeEngineError> {
        if !self.themes.contains_key(id) {
            return Err(ThemeEngineError::ThemeNotFound(id.to_string()));
        }
        self.active_theme_id = Some(id.to_string());
        Ok(())
    }

    pub fn get_active(&self) -> Option<&Theme> {
        self.active_theme_id
            .as_ref()
            .and_then(|id| self.themes.get(id))
    }

    pub fn get_theme(&self, id: &str) -> Option<&Theme> {
        self.themes.get(id)
    }

    // -- listing -----------------------------------------------------------

    pub fn list_themes(&self) -> Vec<&Theme> {
        self.themes.values().collect()
    }

    pub fn themes_by_scheme(&self, scheme: &ColorScheme) -> Vec<&Theme> {
        self.themes
            .values()
            .filter(|t| &t.scheme == scheme)
            .collect()
    }

    // -- duplicate ---------------------------------------------------------

    pub fn duplicate_theme(
        &mut self,
        id: &str,
        new_id: &str,
        new_name: &str,
    ) -> Result<(), ThemeEngineError> {
        let source = self
            .themes
            .get(id)
            .ok_or_else(|| ThemeEngineError::ThemeNotFound(id.to_string()))?
            .clone();

        if self.themes.contains_key(new_id) {
            return Err(ThemeEngineError::DuplicateTheme(new_id.to_string()));
        }

        let new_theme = Theme {
            id: new_id.to_string(),
            name: new_name.to_string(),
            is_builtin: false,
            created_at: Utc::now().to_rfc3339(),
            ..source
        };
        self.themes.insert(new_id.to_string(), new_theme);
        Ok(())
    }

    // -- custom vars -------------------------------------------------------

    pub fn set_custom_var(
        &mut self,
        theme_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), ThemeEngineError> {
        let theme = self
            .themes
            .get_mut(theme_id)
            .ok_or_else(|| ThemeEngineError::ThemeNotFound(theme_id.to_string()))?;
        theme.custom_vars.insert(key.to_string(), value.to_string());
        Ok(())
    }

    // -- export / import ---------------------------------------------------

    pub fn export_theme(&self, id: &str) -> Result<String, ThemeEngineError> {
        let theme = self
            .themes
            .get(id)
            .ok_or_else(|| ThemeEngineError::ThemeNotFound(id.to_string()))?;
        let json = serde_json::to_string_pretty(theme)?;
        Ok(json)
    }

    pub fn import_theme(&mut self, json: &str) -> Result<(), ThemeEngineError> {
        let theme: Theme = serde_json::from_str(json)?;
        self.add_theme(theme)
    }

    // -- stats -------------------------------------------------------------

    pub fn stats(&self) -> ThemeStats {
        let mut schemes_used: HashMap<String, usize> = HashMap::new();
        let mut builtin = 0usize;
        let mut custom = 0usize;

        for theme in self.themes.values() {
            if theme.is_builtin {
                builtin += 1;
            } else {
                custom += 1;
            }
            *schemes_used.entry(theme.scheme.to_string()).or_insert(0) += 1;
        }

        ThemeStats {
            total_themes: self.themes.len(),
            builtin,
            custom,
            active_theme: self.active_theme_id.clone(),
            schemes_used,
        }
    }

    // -- persistence -------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), ThemeEngineError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ThemeEngineError> {
        let data = std::fs::read_to_string(path)?;
        let engine: Self = serde_json::from_str(&data)?;
        Ok(engine)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_theme_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    fn make_custom_theme(id: &str, name: &str) -> Theme {
        Theme {
            id: id.into(),
            name: name.into(),
            description: "A custom theme".into(),
            scheme: ColorScheme::Dark,
            layout: LayoutPreset::Standard,
            colors: ThemeColors::default_dark(),
            custom_vars: HashMap::new(),
            created_at: Utc::now().to_rfc3339(),
            is_builtin: false,
        }
    }

    #[test]
    fn test_register_defaults() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        assert_eq!(engine.themes.len(), 4);
        assert!(engine.get_theme("default_light").is_some());
        assert!(engine.get_theme("default_dark").is_some());
        assert!(engine.get_theme("high_contrast").is_some());
        assert!(engine.get_theme("minimal").is_some());
    }

    #[test]
    fn test_add_theme() {
        let mut engine = ThemeEngine::new();
        let theme = make_custom_theme("ocean", "Ocean");
        assert!(engine.add_theme(theme).is_ok());
        assert!(engine.get_theme("ocean").is_some());
    }

    #[test]
    fn test_add_duplicate_theme() {
        let mut engine = ThemeEngine::new();
        let t1 = make_custom_theme("ocean", "Ocean");
        let t2 = make_custom_theme("ocean", "Ocean 2");
        engine.add_theme(t1).unwrap();
        let err = engine.add_theme(t2).unwrap_err();
        assert!(matches!(err, ThemeEngineError::DuplicateTheme(_)));
    }

    #[test]
    fn test_remove_theme() {
        let mut engine = ThemeEngine::new();
        let theme = make_custom_theme("ocean", "Ocean");
        engine.add_theme(theme).unwrap();
        let removed = engine.remove_theme("ocean").unwrap();
        assert_eq!(removed.id, "ocean");
        assert!(engine.get_theme("ocean").is_none());
    }

    #[test]
    fn test_remove_builtin_fails() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        let err = engine.remove_theme("default_light").unwrap_err();
        assert!(matches!(err, ThemeEngineError::ThemeNotFound(_)));
    }

    #[test]
    fn test_remove_nonexistent_fails() {
        let mut engine = ThemeEngine::new();
        let err = engine.remove_theme("nope").unwrap_err();
        assert!(matches!(err, ThemeEngineError::ThemeNotFound(_)));
    }

    #[test]
    fn test_update_colors() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        let new_colors = ThemeColors {
            primary: "#FF0000".into(),
            ..ThemeColors::default_light()
        };
        engine.update_theme("default_light", new_colors).unwrap();
        let theme = engine.get_theme("default_light").unwrap();
        assert_eq!(theme.colors.primary, "#FF0000");
    }

    #[test]
    fn test_update_nonexistent_fails() {
        let mut engine = ThemeEngine::new();
        let err = engine
            .update_theme("nope", ThemeColors::default_light())
            .unwrap_err();
        assert!(matches!(err, ThemeEngineError::ThemeNotFound(_)));
    }

    #[test]
    fn test_set_and_get_active() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        engine.set_active("default_dark").unwrap();
        let active = engine.get_active().unwrap();
        assert_eq!(active.id, "default_dark");
    }

    #[test]
    fn test_set_active_nonexistent_fails() {
        let mut engine = ThemeEngine::new();
        let err = engine.set_active("nope").unwrap_err();
        assert!(matches!(err, ThemeEngineError::ThemeNotFound(_)));
    }

    #[test]
    fn test_get_active_none_by_default() {
        let engine = ThemeEngine::new();
        assert!(engine.get_active().is_none());
    }

    #[test]
    fn test_list_themes() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        let themes = engine.list_themes();
        assert_eq!(themes.len(), 4);
    }

    #[test]
    fn test_themes_by_scheme() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        let light = engine.themes_by_scheme(&ColorScheme::Light);
        // default_light + minimal
        assert_eq!(light.len(), 2);
        let dark = engine.themes_by_scheme(&ColorScheme::Dark);
        assert_eq!(dark.len(), 1);
    }

    #[test]
    fn test_duplicate_theme() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        engine
            .duplicate_theme("default_dark", "my_dark", "My Dark")
            .unwrap();
        let dup = engine.get_theme("my_dark").unwrap();
        assert_eq!(dup.name, "My Dark");
        assert!(!dup.is_builtin);
        assert_eq!(dup.scheme, ColorScheme::Dark);
    }

    #[test]
    fn test_duplicate_nonexistent_fails() {
        let mut engine = ThemeEngine::new();
        let err = engine.duplicate_theme("nope", "x", "X").unwrap_err();
        assert!(matches!(err, ThemeEngineError::ThemeNotFound(_)));
    }

    #[test]
    fn test_custom_vars() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        engine
            .set_custom_var("default_light", "border_radius", "8px")
            .unwrap();
        let theme = engine.get_theme("default_light").unwrap();
        assert_eq!(theme.custom_vars.get("border_radius").unwrap(), "8px");
    }

    #[test]
    fn test_custom_var_nonexistent_theme() {
        let mut engine = ThemeEngine::new();
        let err = engine.set_custom_var("nope", "k", "v").unwrap_err();
        assert!(matches!(err, ThemeEngineError::ThemeNotFound(_)));
    }

    #[test]
    fn test_export_theme() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        let json = engine.export_theme("default_light").unwrap();
        assert!(json.contains("default_light"));
        assert!(json.contains("primary"));
    }

    #[test]
    fn test_import_theme() {
        let mut engine = ThemeEngine::new();
        let theme = make_custom_theme("imported", "Imported");
        let json = serde_json::to_string(&theme).unwrap();
        engine.import_theme(&json).unwrap();
        assert!(engine.get_theme("imported").is_some());
    }

    #[test]
    fn test_stats() {
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        engine
            .add_theme(make_custom_theme("extra", "Extra"))
            .unwrap();
        engine.set_active("default_light").unwrap();

        let stats = engine.stats();
        assert_eq!(stats.total_themes, 5);
        assert_eq!(stats.builtin, 4);
        assert_eq!(stats.custom, 1);
        assert_eq!(stats.active_theme, Some("default_light".into()));
        assert!(stats.schemes_used.contains_key("Light"));
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = temp_path("roundtrip.json");
        let mut engine = ThemeEngine::new();
        engine.register_defaults();
        engine.set_active("default_dark").unwrap();
        engine.save(&path).unwrap();

        let loaded = ThemeEngine::load(&path).unwrap();
        assert_eq!(loaded.themes.len(), 4);
        assert_eq!(loaded.active_theme_id, Some("default_dark".into()));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("nonexistent_file.json");
        let _ = std::fs::remove_file(&path); // ensure absent
        let engine = ThemeEngine::load_or_default(&path);
        assert!(engine.themes.is_empty());
        assert!(engine.active_theme_id.is_none());
    }
}
