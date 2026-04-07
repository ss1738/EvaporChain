use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum HelpSystemError {
    #[error("Topic not found: {0}")]
    TopicNotFound(String),
    #[error("Duplicate topic: {0}")]
    DuplicateTopic(String),
    #[error("FAQ not found: {0}")]
    FaqNotFound(String),
    #[error("Tutorial not found: {0}")]
    TutorialNotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HelpCategory2 {
    GettingStarted,
    Accounts,
    Transactions,
    Energy,
    Security,
    DeFi,
    Advanced,
    Troubleshooting,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Difficulty {
    Beginner,
    Intermediate,
    Advanced,
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpTopic {
    pub id: String,
    pub title: String,
    pub content: String,
    pub category: HelpCategory2,
    pub tags: Vec<String>,
    pub related: Vec<String>,
    pub views: u64,
    pub last_viewed: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaqEntry {
    pub id: String,
    pub question: String,
    pub answer: String,
    pub category: HelpCategory2,
    pub helpful_count: u32,
    pub not_helpful_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tutorial {
    pub id: String,
    pub title: String,
    pub steps: Vec<TutorialStep>,
    pub difficulty: Difficulty,
    pub estimated_minutes: u32,
    pub completed: bool,
    pub category: HelpCategory2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TutorialStep {
    pub order: u32,
    pub title: String,
    pub content: String,
    pub command_example: Option<String>,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorExplanation {
    pub error_code: String,
    pub title: String,
    pub explanation: String,
    pub solution: String,
    pub related_topics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpStats2 {
    pub total_topics: usize,
    pub total_faqs: usize,
    pub total_tutorials: usize,
    pub completed_tutorials: usize,
    pub total_views: u64,
    pub most_viewed: Option<String>,
    pub error_explanations: usize,
}

// ---------------------------------------------------------------------------
// Main struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HelpSystem {
    pub topics: HashMap<String, HelpTopic>,
    pub faqs: Vec<FaqEntry>,
    pub tutorials: HashMap<String, Tutorial>,
    pub error_explanations: HashMap<String, ErrorExplanation>,
}

impl HelpSystem {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Topics -------------------------------------------------------------

    pub fn add_topic(&mut self, topic: HelpTopic) -> Result<(), HelpSystemError> {
        if self.topics.contains_key(&topic.id) {
            return Err(HelpSystemError::DuplicateTopic(topic.id.clone()));
        }
        self.topics.insert(topic.id.clone(), topic);
        Ok(())
    }

    pub fn remove_topic(&mut self, id: &str) -> Result<HelpTopic, HelpSystemError> {
        self.topics
            .remove(id)
            .ok_or_else(|| HelpSystemError::TopicNotFound(id.to_string()))
    }

    pub fn view_topic(&mut self, id: &str) -> Result<&HelpTopic, HelpSystemError> {
        let topic = self
            .topics
            .get_mut(id)
            .ok_or_else(|| HelpSystemError::TopicNotFound(id.to_string()))?;
        topic.views += 1;
        topic.last_viewed = Some(Utc::now().to_rfc3339());
        // Re-borrow as immutable
        Ok(self.topics.get(id).unwrap())
    }

    pub fn search_topics(&self, query: &str) -> Vec<&HelpTopic> {
        let q = query.to_lowercase();
        self.topics
            .values()
            .filter(|t| {
                t.title.to_lowercase().contains(&q)
                    || t.content.to_lowercase().contains(&q)
                    || t.tags.iter().any(|tag| tag.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub fn topics_by_category(&self, cat: &HelpCategory2) -> Vec<&HelpTopic> {
        self.topics
            .values()
            .filter(|t| &t.category == cat)
            .collect()
    }

    pub fn related_topics(&self, id: &str) -> Result<Vec<&HelpTopic>, HelpSystemError> {
        let topic = self
            .topics
            .get(id)
            .ok_or_else(|| HelpSystemError::TopicNotFound(id.to_string()))?;
        let related: Vec<&HelpTopic> = topic
            .related
            .iter()
            .filter_map(|rid| self.topics.get(rid))
            .collect();
        Ok(related)
    }

    // -- FAQs ---------------------------------------------------------------

    pub fn add_faq(&mut self, faq: FaqEntry) {
        self.faqs.push(faq);
    }

    pub fn search_faq(&self, query: &str) -> Vec<&FaqEntry> {
        let q = query.to_lowercase();
        self.faqs
            .iter()
            .filter(|f| {
                f.question.to_lowercase().contains(&q)
                    || f.answer.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn rate_faq(&mut self, id: &str, helpful: bool) -> Result<(), HelpSystemError> {
        let faq = self
            .faqs
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or_else(|| HelpSystemError::FaqNotFound(id.to_string()))?;
        if helpful {
            faq.helpful_count += 1;
        } else {
            faq.not_helpful_count += 1;
        }
        Ok(())
    }

    // -- Tutorials ----------------------------------------------------------

    pub fn add_tutorial(&mut self, tutorial: Tutorial) -> Result<(), HelpSystemError> {
        if self.tutorials.contains_key(&tutorial.id) {
            return Err(HelpSystemError::DuplicateTopic(tutorial.id.clone()));
        }
        self.tutorials.insert(tutorial.id.clone(), tutorial);
        Ok(())
    }

    pub fn complete_tutorial_step(
        &mut self,
        tutorial_id: &str,
        step_order: u32,
    ) -> Result<(), HelpSystemError> {
        let tutorial = self
            .tutorials
            .get_mut(tutorial_id)
            .ok_or_else(|| HelpSystemError::TutorialNotFound(tutorial_id.to_string()))?;
        let step = tutorial
            .steps
            .iter_mut()
            .find(|s| s.order == step_order)
            .ok_or_else(|| {
                HelpSystemError::TutorialNotFound(format!(
                    "step {} in tutorial {}",
                    step_order, tutorial_id
                ))
            })?;
        step.completed = true;
        // Check if all steps are now completed
        if tutorial.steps.iter().all(|s| s.completed) {
            tutorial.completed = true;
        }
        Ok(())
    }

    pub fn tutorials_by_difficulty(&self, diff: &Difficulty) -> Vec<&Tutorial> {
        self.tutorials
            .values()
            .filter(|t| &t.difficulty == diff)
            .collect()
    }

    // -- Error explanations -------------------------------------------------

    pub fn add_error_explanation(&mut self, explanation: ErrorExplanation) {
        self.error_explanations
            .insert(explanation.error_code.clone(), explanation);
    }

    pub fn explain_error(&self, code: &str) -> Option<&ErrorExplanation> {
        self.error_explanations.get(code)
    }

    // -- Aggregation --------------------------------------------------------

    pub fn popular_topics(&self, n: usize) -> Vec<&HelpTopic> {
        let mut topics: Vec<&HelpTopic> = self.topics.values().collect();
        topics.sort_by(|a, b| b.views.cmp(&a.views));
        topics.truncate(n);
        topics
    }

    pub fn stats(&self) -> HelpStats2 {
        let total_views: u64 = self.topics.values().map(|t| t.views).sum();
        let most_viewed = self
            .topics
            .values()
            .max_by_key(|t| t.views)
            .map(|t| t.id.clone());
        HelpStats2 {
            total_topics: self.topics.len(),
            total_faqs: self.faqs.len(),
            total_tutorials: self.tutorials.len(),
            completed_tutorials: self.tutorials.values().filter(|t| t.completed).count(),
            total_views,
            most_viewed,
            error_explanations: self.error_explanations.len(),
        }
    }

    // -- Persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), HelpSystemError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, HelpSystemError> {
        let data = std::fs::read_to_string(path)?;
        let system: Self = serde_json::from_str(&data)?;
        Ok(system)
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
    use std::env::temp_dir;
    use std::process;

    fn test_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("evaporchain_help_test_{}_{}.json", process::id(), name))
    }

    fn make_topic(id: &str, category: HelpCategory2) -> HelpTopic {
        HelpTopic {
            id: id.to_string(),
            title: format!("Title {}", id),
            content: format!("Content for {}", id),
            category,
            tags: vec!["wallet".to_string()],
            related: vec![],
            views: 0,
            last_viewed: None,
        }
    }

    fn make_faq(id: &str, question: &str, answer: &str) -> FaqEntry {
        FaqEntry {
            id: id.to_string(),
            question: question.to_string(),
            answer: answer.to_string(),
            category: HelpCategory2::GettingStarted,
            helpful_count: 0,
            not_helpful_count: 0,
        }
    }

    fn make_tutorial(id: &str, difficulty: Difficulty, steps: usize) -> Tutorial {
        let steps_vec: Vec<TutorialStep> = (1..=steps as u32)
            .map(|i| TutorialStep {
                order: i,
                title: format!("Step {}", i),
                content: format!("Do step {}", i),
                command_example: Some(format!("cmd {}", i)),
                completed: false,
            })
            .collect();
        Tutorial {
            id: id.to_string(),
            title: format!("Tutorial {}", id),
            steps: steps_vec,
            difficulty,
            estimated_minutes: 10,
            completed: false,
            category: HelpCategory2::GettingStarted,
        }
    }

    #[test]
    fn test_add_topic() {
        let mut sys = HelpSystem::new();
        let topic = make_topic("t1", HelpCategory2::Accounts);
        assert!(sys.add_topic(topic).is_ok());
        assert_eq!(sys.topics.len(), 1);
    }

    #[test]
    fn test_duplicate_topic() {
        let mut sys = HelpSystem::new();
        let topic = make_topic("t1", HelpCategory2::Accounts);
        sys.add_topic(topic.clone()).unwrap();
        let result = sys.add_topic(topic);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), HelpSystemError::DuplicateTopic(_)));
    }

    #[test]
    fn test_remove_topic() {
        let mut sys = HelpSystem::new();
        sys.add_topic(make_topic("t1", HelpCategory2::Accounts)).unwrap();
        let removed = sys.remove_topic("t1").unwrap();
        assert_eq!(removed.id, "t1");
        assert!(sys.topics.is_empty());
    }

    #[test]
    fn test_remove_topic_not_found() {
        let mut sys = HelpSystem::new();
        let result = sys.remove_topic("nonexistent");
        assert!(matches!(result.unwrap_err(), HelpSystemError::TopicNotFound(_)));
    }

    #[test]
    fn test_view_topic_increments_views() {
        let mut sys = HelpSystem::new();
        sys.add_topic(make_topic("t1", HelpCategory2::Security)).unwrap();
        let topic = sys.view_topic("t1").unwrap();
        assert_eq!(topic.views, 1);
        assert!(topic.last_viewed.is_some());
        let topic2 = sys.view_topic("t1").unwrap();
        assert_eq!(topic2.views, 2);
    }

    #[test]
    fn test_view_topic_not_found() {
        let mut sys = HelpSystem::new();
        let result = sys.view_topic("missing");
        assert!(matches!(result.unwrap_err(), HelpSystemError::TopicNotFound(_)));
    }

    #[test]
    fn test_search_topics_by_title() {
        let mut sys = HelpSystem::new();
        let mut topic = make_topic("t1", HelpCategory2::Transactions);
        topic.title = "How to send tokens".to_string();
        sys.add_topic(topic).unwrap();
        let results = sys.search_topics("send");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_topics_by_content() {
        let mut sys = HelpSystem::new();
        let mut topic = make_topic("t1", HelpCategory2::Transactions);
        topic.content = "Use the bridge to transfer assets cross-chain".to_string();
        sys.add_topic(topic).unwrap();
        let results = sys.search_topics("bridge");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_topics_by_tag() {
        let mut sys = HelpSystem::new();
        let mut topic = make_topic("t1", HelpCategory2::Energy);
        topic.tags = vec!["staking".to_string(), "rewards".to_string()];
        sys.add_topic(topic).unwrap();
        let results = sys.search_topics("staking");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_topics_by_category() {
        let mut sys = HelpSystem::new();
        sys.add_topic(make_topic("t1", HelpCategory2::DeFi)).unwrap();
        sys.add_topic(make_topic("t2", HelpCategory2::Security)).unwrap();
        sys.add_topic(make_topic("t3", HelpCategory2::DeFi)).unwrap();
        let defi = sys.topics_by_category(&HelpCategory2::DeFi);
        assert_eq!(defi.len(), 2);
    }

    #[test]
    fn test_related_topics() {
        let mut sys = HelpSystem::new();
        let mut t1 = make_topic("t1", HelpCategory2::Accounts);
        t1.related = vec!["t2".to_string(), "t3".to_string()];
        sys.add_topic(t1).unwrap();
        sys.add_topic(make_topic("t2", HelpCategory2::Accounts)).unwrap();
        // t3 doesn't exist — should be silently skipped
        let related = sys.related_topics("t1").unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, "t2");
    }

    #[test]
    fn test_related_topics_not_found() {
        let sys = HelpSystem::new();
        assert!(matches!(
            sys.related_topics("nope").unwrap_err(),
            HelpSystemError::TopicNotFound(_)
        ));
    }

    #[test]
    fn test_add_and_search_faq() {
        let mut sys = HelpSystem::new();
        sys.add_faq(make_faq("f1", "How do I stake?", "Go to the staking page."));
        sys.add_faq(make_faq("f2", "What is energy?", "Energy powers transactions."));
        let results = sys.search_faq("stake");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "f1");
    }

    #[test]
    fn test_rate_faq_helpful() {
        let mut sys = HelpSystem::new();
        sys.add_faq(make_faq("f1", "Q?", "A."));
        sys.rate_faq("f1", true).unwrap();
        sys.rate_faq("f1", true).unwrap();
        sys.rate_faq("f1", false).unwrap();
        let faq = sys.faqs.iter().find(|f| f.id == "f1").unwrap();
        assert_eq!(faq.helpful_count, 2);
        assert_eq!(faq.not_helpful_count, 1);
    }

    #[test]
    fn test_rate_faq_not_found() {
        let mut sys = HelpSystem::new();
        assert!(matches!(
            sys.rate_faq("missing", true).unwrap_err(),
            HelpSystemError::FaqNotFound(_)
        ));
    }

    #[test]
    fn test_add_tutorial() {
        let mut sys = HelpSystem::new();
        assert!(sys.add_tutorial(make_tutorial("tut1", Difficulty::Beginner, 3)).is_ok());
        assert_eq!(sys.tutorials.len(), 1);
    }

    #[test]
    fn test_add_duplicate_tutorial() {
        let mut sys = HelpSystem::new();
        sys.add_tutorial(make_tutorial("tut1", Difficulty::Beginner, 2)).unwrap();
        let result = sys.add_tutorial(make_tutorial("tut1", Difficulty::Advanced, 1));
        assert!(result.is_err());
    }

    #[test]
    fn test_complete_tutorial_steps() {
        let mut sys = HelpSystem::new();
        sys.add_tutorial(make_tutorial("tut1", Difficulty::Intermediate, 2)).unwrap();
        sys.complete_tutorial_step("tut1", 1).unwrap();
        assert!(!sys.tutorials["tut1"].completed);
        sys.complete_tutorial_step("tut1", 2).unwrap();
        assert!(sys.tutorials["tut1"].completed);
    }

    #[test]
    fn test_complete_tutorial_step_not_found() {
        let mut sys = HelpSystem::new();
        let result = sys.complete_tutorial_step("nope", 1);
        assert!(matches!(result.unwrap_err(), HelpSystemError::TutorialNotFound(_)));
    }

    #[test]
    fn test_tutorials_by_difficulty() {
        let mut sys = HelpSystem::new();
        sys.add_tutorial(make_tutorial("b1", Difficulty::Beginner, 1)).unwrap();
        sys.add_tutorial(make_tutorial("a1", Difficulty::Advanced, 1)).unwrap();
        sys.add_tutorial(make_tutorial("b2", Difficulty::Beginner, 2)).unwrap();
        let beginners = sys.tutorials_by_difficulty(&Difficulty::Beginner);
        assert_eq!(beginners.len(), 2);
    }

    #[test]
    fn test_error_explanation() {
        let mut sys = HelpSystem::new();
        sys.add_error_explanation(ErrorExplanation {
            error_code: "E001".to_string(),
            title: "Insufficient energy".to_string(),
            explanation: "Not enough energy to complete the transaction.".to_string(),
            solution: "Stake more tokens or wait for energy regeneration.".to_string(),
            related_topics: vec!["t1".to_string()],
        });
        let expl = sys.explain_error("E001");
        assert!(expl.is_some());
        assert_eq!(expl.unwrap().title, "Insufficient energy");
        assert!(sys.explain_error("E999").is_none());
    }

    #[test]
    fn test_popular_topics() {
        let mut sys = HelpSystem::new();
        let mut t1 = make_topic("t1", HelpCategory2::Accounts);
        t1.views = 100;
        let mut t2 = make_topic("t2", HelpCategory2::Security);
        t2.views = 50;
        let mut t3 = make_topic("t3", HelpCategory2::DeFi);
        t3.views = 200;
        sys.add_topic(t1).unwrap();
        sys.add_topic(t2).unwrap();
        sys.add_topic(t3).unwrap();
        let popular = sys.popular_topics(2);
        assert_eq!(popular.len(), 2);
        assert_eq!(popular[0].id, "t3");
        assert_eq!(popular[1].id, "t1");
    }

    #[test]
    fn test_stats() {
        let mut sys = HelpSystem::new();
        let mut t1 = make_topic("t1", HelpCategory2::Accounts);
        t1.views = 10;
        sys.add_topic(t1).unwrap();
        sys.add_faq(make_faq("f1", "Q", "A"));
        let mut tut = make_tutorial("tut1", Difficulty::Beginner, 1);
        tut.completed = true;
        sys.add_tutorial(tut).unwrap();
        sys.add_tutorial(make_tutorial("tut2", Difficulty::Advanced, 2)).unwrap();
        sys.add_error_explanation(ErrorExplanation {
            error_code: "E1".to_string(),
            title: "Err".to_string(),
            explanation: "Expl".to_string(),
            solution: "Sol".to_string(),
            related_topics: vec![],
        });
        let s = sys.stats();
        assert_eq!(s.total_topics, 1);
        assert_eq!(s.total_faqs, 1);
        assert_eq!(s.total_tutorials, 2);
        assert_eq!(s.completed_tutorials, 1);
        assert_eq!(s.total_views, 10);
        assert_eq!(s.most_viewed, Some("t1".to_string()));
        assert_eq!(s.error_explanations, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut sys = HelpSystem::new();
        sys.add_topic(make_topic("t1", HelpCategory2::Transactions)).unwrap();
        sys.add_faq(make_faq("f1", "Q?", "A."));
        sys.add_tutorial(make_tutorial("tut1", Difficulty::Beginner, 2)).unwrap();
        sys.save(&path).unwrap();

        let loaded = HelpSystem::load(&path).unwrap();
        assert_eq!(loaded.topics.len(), 1);
        assert_eq!(loaded.faqs.len(), 1);
        assert_eq!(loaded.tutorials.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("load_or_default_missing");
        let _ = std::fs::remove_file(&path); // ensure it doesn't exist
        let sys = HelpSystem::load_or_default(&path);
        assert!(sys.topics.is_empty());
        assert!(sys.faqs.is_empty());
    }
}
