use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CommandPaletteError {
    #[error("command not found: {0}")]
    CommandNotFound(String),
    #[error("duplicate alias: {0}")]
    DuplicateAlias(String),
    #[error("alias not found: {0}")]
    AliasNotFound(String),
    #[error("duplicate command: {0}")]
    DuplicateCommand(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CommandCategory {
    Account,
    Transaction,
    Energy,
    Staking,
    DeFi,
    Security,
    Network,
    Utility,
    Custom(String),
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaletteCommand {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: CommandCategory,
    pub usage: String,
    pub examples: Vec<String>,
    pub tags: Vec<String>,
    pub use_count: u64,
    pub last_used: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAlias {
    pub alias: String,
    pub command_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub command: PaletteCommand,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PaletteStats {
    pub total_commands: usize,
    pub total_aliases: usize,
    pub total_uses: u64,
    pub most_used: Option<String>,
    pub categories: HashMap<String, usize>,
    pub recent_commands: Vec<String>,
}

// ---------------------------------------------------------------------------
// CommandPalette
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPalette {
    pub commands: HashMap<String, PaletteCommand>,
    pub aliases: HashMap<String, CommandAlias>,
    pub recent: Vec<String>,
    pub max_recent: usize,
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self {
            commands: HashMap::new(),
            aliases: HashMap::new(),
            recent: Vec::new(),
            max_recent: 50,
        }
    }
}

impl CommandPalette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_command(&mut self, cmd: PaletteCommand) -> Result<(), CommandPaletteError> {
        if self.commands.contains_key(&cmd.id) {
            return Err(CommandPaletteError::DuplicateCommand(cmd.id.clone()));
        }
        self.commands.insert(cmd.id.clone(), cmd);
        Ok(())
    }

    pub fn remove_command(&mut self, id: &str) -> Result<PaletteCommand, CommandPaletteError> {
        self.commands
            .remove(id)
            .ok_or_else(|| CommandPaletteError::CommandNotFound(id.to_string()))
    }

    pub fn add_alias(&mut self, alias: &str, command_id: &str) -> Result<(), CommandPaletteError> {
        if !self.commands.contains_key(command_id) {
            return Err(CommandPaletteError::CommandNotFound(command_id.to_string()));
        }
        if self.aliases.contains_key(alias) {
            return Err(CommandPaletteError::DuplicateAlias(alias.to_string()));
        }
        self.aliases.insert(
            alias.to_string(),
            CommandAlias {
                alias: alias.to_string(),
                command_id: command_id.to_string(),
                created_at: Utc::now().to_rfc3339(),
            },
        );
        Ok(())
    }

    pub fn remove_alias(&mut self, alias: &str) -> Result<CommandAlias, CommandPaletteError> {
        self.aliases
            .remove(alias)
            .ok_or_else(|| CommandPaletteError::AliasNotFound(alias.to_string()))
    }

    pub fn resolve_alias(&self, alias: &str) -> Option<&str> {
        self.aliases.get(alias).map(|a| a.command_id.as_str())
    }

    pub fn record_use(&mut self, id: &str) -> Result<(), CommandPaletteError> {
        let cmd = self
            .commands
            .get_mut(id)
            .ok_or_else(|| CommandPaletteError::CommandNotFound(id.to_string()))?;
        cmd.use_count += 1;
        cmd.last_used = Some(Utc::now().to_rfc3339());

        // Update recent list — remove old occurrence then push to front.
        self.recent.retain(|r| r != id);
        self.recent.insert(0, id.to_string());
        if self.recent.len() > self.max_recent {
            self.recent.truncate(self.max_recent);
        }
        Ok(())
    }

    pub fn fuzzy_search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.to_lowercase();
        let mut results: Vec<SearchResult> = Vec::new();

        for cmd in self.commands.values() {
            let mut score: f64 = 0.0;
            let name_lower = cmd.name.to_lowercase();

            if name_lower == q {
                score = 100.0;
            } else if name_lower.contains(&q) {
                score = f64::max(score, 70.0);
            }

            for tag in &cmd.tags {
                if tag.to_lowercase() == q {
                    score = f64::max(score, 50.0);
                }
            }

            if cmd.description.to_lowercase().contains(&q) && score < 30.0 {
                score = f64::max(score, 30.0);
            }

            if score > 0.0 {
                results.push(SearchResult {
                    command: cmd.clone(),
                    score,
                });
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    pub fn commands_by_category(&self, cat: &CommandCategory) -> Vec<&PaletteCommand> {
        self.commands
            .values()
            .filter(|c| &c.category == cat)
            .collect()
    }

    pub fn recent_commands(&self, n: usize) -> Vec<&PaletteCommand> {
        self.recent
            .iter()
            .take(n)
            .filter_map(|id| self.commands.get(id))
            .collect()
    }

    pub fn most_used(&self, n: usize) -> Vec<&PaletteCommand> {
        let mut cmds: Vec<&PaletteCommand> = self.commands.values().collect();
        cmds.sort_by_key(|a| std::cmp::Reverse(a.use_count));
        cmds.into_iter().take(n).collect()
    }

    pub fn search_by_tag(&self, tag: &str) -> Vec<&PaletteCommand> {
        let t = tag.to_lowercase();
        self.commands
            .values()
            .filter(|c| c.tags.iter().any(|tg| tg.to_lowercase() == t))
            .collect()
    }

    pub fn get_command(&self, id: &str) -> Option<&PaletteCommand> {
        self.commands.get(id)
    }

    pub fn all_aliases(&self) -> Vec<&CommandAlias> {
        self.aliases.values().collect()
    }

    pub fn stats(&self) -> PaletteStats {
        let total_uses: u64 = self.commands.values().map(|c| c.use_count).sum();
        let most_used = self
            .commands
            .values()
            .max_by_key(|c| c.use_count)
            .filter(|c| c.use_count > 0)
            .map(|c| c.id.clone());

        let mut categories: HashMap<String, usize> = HashMap::new();
        for cmd in self.commands.values() {
            let key = format!("{:?}", cmd.category);
            *categories.entry(key).or_insert(0) += 1;
        }

        PaletteStats {
            total_commands: self.commands.len(),
            total_aliases: self.aliases.len(),
            total_uses,
            most_used,
            categories,
            recent_commands: self.recent.clone(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), CommandPaletteError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, CommandPaletteError> {
        let data = std::fs::read_to_string(path)?;
        let palette: Self = serde_json::from_str(&data)?;
        Ok(palette)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helper for tests
// ---------------------------------------------------------------------------

#[cfg(test)]
fn make_command(
    id: &str,
    name: &str,
    desc: &str,
    cat: CommandCategory,
    tags: &[&str],
) -> PaletteCommand {
    PaletteCommand {
        id: id.to_string(),
        name: name.to_string(),
        description: desc.to_string(),
        category: cat,
        usage: format!("{} [args]", name),
        examples: vec![format!("{} --help", name)],
        tags: tags.iter().map(|t| t.to_string()).collect(),
        use_count: 0,
        last_used: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process;

    fn test_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("evaporchain_cp_{}_{}.json", process::id(), name))
    }

    fn sample_command(id: &str) -> PaletteCommand {
        make_command(
            id,
            &format!("cmd_{}", id),
            &format!("Description for {}", id),
            CommandCategory::Utility,
            &["general"],
        )
    }

    #[test]
    fn test_register_command() {
        let mut p = CommandPalette::new();
        let cmd = sample_command("send");
        assert!(p.register_command(cmd).is_ok());
        assert!(p.get_command("send").is_some());
    }

    #[test]
    fn test_register_duplicate_command() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("dup")).unwrap();
        let res = p.register_command(sample_command("dup"));
        assert!(matches!(res, Err(CommandPaletteError::DuplicateCommand(_))));
    }

    #[test]
    fn test_remove_command() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("rm")).unwrap();
        let removed = p.remove_command("rm").unwrap();
        assert_eq!(removed.id, "rm");
        assert!(p.get_command("rm").is_none());
    }

    #[test]
    fn test_remove_command_not_found() {
        let mut p = CommandPalette::new();
        assert!(matches!(
            p.remove_command("nope"),
            Err(CommandPaletteError::CommandNotFound(_))
        ));
    }

    #[test]
    fn test_add_alias() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("transfer")).unwrap();
        assert!(p.add_alias("tx", "transfer").is_ok());
    }

    #[test]
    fn test_add_alias_missing_command() {
        let mut p = CommandPalette::new();
        let res = p.add_alias("tx", "nonexistent");
        assert!(matches!(res, Err(CommandPaletteError::CommandNotFound(_))));
    }

    #[test]
    fn test_add_duplicate_alias() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("transfer")).unwrap();
        p.add_alias("tx", "transfer").unwrap();
        let res = p.add_alias("tx", "transfer");
        assert!(matches!(res, Err(CommandPaletteError::DuplicateAlias(_))));
    }

    #[test]
    fn test_remove_alias() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("transfer")).unwrap();
        p.add_alias("tx", "transfer").unwrap();
        let removed = p.remove_alias("tx").unwrap();
        assert_eq!(removed.alias, "tx");
        assert!(p.resolve_alias("tx").is_none());
    }

    #[test]
    fn test_remove_alias_not_found() {
        let mut p = CommandPalette::new();
        assert!(matches!(
            p.remove_alias("nope"),
            Err(CommandPaletteError::AliasNotFound(_))
        ));
    }

    #[test]
    fn test_resolve_alias() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("stake")).unwrap();
        p.add_alias("s", "stake").unwrap();
        assert_eq!(p.resolve_alias("s"), Some("stake"));
        assert_eq!(p.resolve_alias("missing"), None);
    }

    #[test]
    fn test_record_use() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("send")).unwrap();
        p.record_use("send").unwrap();
        let cmd = p.get_command("send").unwrap();
        assert_eq!(cmd.use_count, 1);
        assert!(cmd.last_used.is_some());
    }

    #[test]
    fn test_record_use_not_found() {
        let mut p = CommandPalette::new();
        assert!(matches!(
            p.record_use("ghost"),
            Err(CommandPaletteError::CommandNotFound(_))
        ));
    }

    #[test]
    fn test_fuzzy_search_exact_name() {
        let mut p = CommandPalette::new();
        let cmd = make_command(
            "send",
            "send",
            "Send tokens",
            CommandCategory::Transaction,
            &["transfer"],
        );
        p.register_command(cmd).unwrap();
        let results = p.fuzzy_search("send");
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_fuzzy_search_partial_name() {
        let mut p = CommandPalette::new();
        let cmd = make_command(
            "send_tokens",
            "send_tokens",
            "Broadcast",
            CommandCategory::Transaction,
            &[],
        );
        p.register_command(cmd).unwrap();
        let results = p.fuzzy_search("send");
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_fuzzy_search_tag_match() {
        let mut p = CommandPalette::new();
        let cmd = make_command(
            "x",
            "unrelated_name",
            "Unrelated desc",
            CommandCategory::Utility,
            &["wallet"],
        );
        p.register_command(cmd).unwrap();
        let results = p.fuzzy_search("wallet");
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_fuzzy_search_description_match() {
        let mut p = CommandPalette::new();
        let cmd = make_command(
            "y",
            "xyz",
            "Manage your energy credits",
            CommandCategory::Energy,
            &[],
        );
        p.register_command(cmd).unwrap();
        let results = p.fuzzy_search("energy");
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_fuzzy_search_no_match() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("abc")).unwrap();
        let results = p.fuzzy_search("zzzzzzz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_commands_by_category() {
        let mut p = CommandPalette::new();
        p.register_command(make_command("a", "a", "a", CommandCategory::Staking, &[]))
            .unwrap();
        p.register_command(make_command("b", "b", "b", CommandCategory::Utility, &[]))
            .unwrap();
        let staking = p.commands_by_category(&CommandCategory::Staking);
        assert_eq!(staking.len(), 1);
        assert_eq!(staking[0].id, "a");
    }

    #[test]
    fn test_recent_commands() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("r1")).unwrap();
        p.register_command(sample_command("r2")).unwrap();
        p.record_use("r1").unwrap();
        p.record_use("r2").unwrap();
        let recent = p.recent_commands(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "r2");
    }

    #[test]
    fn test_most_used() {
        let mut p = CommandPalette::new();
        p.register_command(sample_command("m1")).unwrap();
        p.register_command(sample_command("m2")).unwrap();
        p.record_use("m1").unwrap();
        p.record_use("m2").unwrap();
        p.record_use("m2").unwrap();
        let top = p.most_used(1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].id, "m2");
    }

    #[test]
    fn test_search_by_tag() {
        let mut p = CommandPalette::new();
        let cmd = make_command("t1", "t1", "d", CommandCategory::Utility, &["defi", "swap"]);
        p.register_command(cmd).unwrap();
        let found = p.search_by_tag("defi");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "t1");
        assert!(p.search_by_tag("notag").is_empty());
    }

    #[test]
    fn test_stats() {
        let mut p = CommandPalette::new();
        p.register_command(make_command("s1", "s1", "d", CommandCategory::Account, &[]))
            .unwrap();
        p.register_command(make_command("s2", "s2", "d", CommandCategory::Account, &[]))
            .unwrap();
        p.add_alias("a1", "s1").unwrap();
        p.record_use("s1").unwrap();
        p.record_use("s1").unwrap();
        p.record_use("s2").unwrap();

        let stats = p.stats();
        assert_eq!(stats.total_commands, 2);
        assert_eq!(stats.total_aliases, 1);
        assert_eq!(stats.total_uses, 3);
        assert_eq!(stats.most_used, Some("s1".to_string()));
        assert_eq!(stats.recent_commands.len(), 2);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut p = CommandPalette::new();
        p.register_command(sample_command("persist")).unwrap();
        p.add_alias("per", "persist").unwrap();
        p.record_use("persist").unwrap();
        p.save(&path).unwrap();

        let loaded = CommandPalette::load(&path).unwrap();
        assert!(loaded.get_command("persist").is_some());
        assert_eq!(loaded.get_command("persist").unwrap().use_count, 1);
        assert_eq!(loaded.resolve_alias("per"), Some("persist"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent_load_or_default");
        let _ = std::fs::remove_file(&path);
        let palette = CommandPalette::load_or_default(&path);
        assert_eq!(palette.commands.len(), 0);
        assert_eq!(palette.max_recent, 50);
    }
}
