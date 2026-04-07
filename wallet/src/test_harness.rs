use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum TestHarnessError {
    #[error("fixture not found: {0}")]
    FixtureNotFound(String),
    #[error("duplicate fixture: {0}")]
    DuplicateFixture(String),
    #[error("mock not found: {0}")]
    MockNotFound(String),
    #[error("assertion failed: {0}")]
    AssertionFailed(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FixtureType {
    Account,
    Transaction,
    Block,
    Token,
    NFT,
    Contract,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MockBehavior {
    ReturnValue(String),
    ReturnError(String),
    Delay(u64),
    Sequence(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TestResult2 {
    Pass,
    Fail(String),
    Skip(String),
    Error(String),
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFixture {
    pub id: String,
    pub fixture_type: FixtureType,
    pub name: String,
    pub data: HashMap<String, String>,
    pub created_at: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockBuilder {
    pub id: String,
    pub name: String,
    pub behavior: MockBehavior,
    pub call_count: u64,
    pub max_calls: Option<u64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    pub id: String,
    pub name: String,
    pub description: String,
    pub fixture_ids: Vec<String>,
    pub mock_ids: Vec<String>,
    pub result: Option<TestResult2>,
    pub executed_at: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSuite2 {
    pub id: String,
    pub name: String,
    pub cases: Vec<String>,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub errors: u32,
    pub executed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessStats {
    pub total_fixtures: usize,
    pub total_mocks: usize,
    pub total_cases: usize,
    pub total_suites: usize,
    pub pass_rate: f64,
    pub total_executions: u64,
}

// ---------------------------------------------------------------------------
// Main struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestHarness {
    pub fixtures: HashMap<String, TestFixture>,
    pub mocks: HashMap<String, MockBuilder>,
    pub cases: HashMap<String, TestCase>,
    pub suites: HashMap<String, TestSuite2>,
}

impl TestHarness {
    pub fn new() -> Self {
        Self::default()
    }

    // -- fixtures -----------------------------------------------------------

    pub fn add_fixture(&mut self, fixture: TestFixture) -> Result<(), TestHarnessError> {
        if self.fixtures.contains_key(&fixture.id) {
            return Err(TestHarnessError::DuplicateFixture(fixture.id.clone()));
        }
        self.fixtures.insert(fixture.id.clone(), fixture);
        Ok(())
    }

    pub fn remove_fixture(&mut self, id: &str) -> Result<TestFixture, TestHarnessError> {
        self.fixtures
            .remove(id)
            .ok_or_else(|| TestHarnessError::FixtureNotFound(id.to_string()))
    }

    pub fn get_fixture(&self, id: &str) -> Option<&TestFixture> {
        self.fixtures.get(id)
    }

    pub fn fixtures_by_type(&self, ft: &FixtureType) -> Vec<&TestFixture> {
        self.fixtures
            .values()
            .filter(|f| &f.fixture_type == ft)
            .collect()
    }

    pub fn fixtures_by_tag(&self, tag: &str) -> Vec<&TestFixture> {
        self.fixtures
            .values()
            .filter(|f| f.tags.iter().any(|t| t == tag))
            .collect()
    }

    // -- mocks --------------------------------------------------------------

    pub fn create_mock(&mut self, mock: MockBuilder) -> Result<(), TestHarnessError> {
        if self.mocks.contains_key(&mock.id) {
            return Err(TestHarnessError::DuplicateFixture(mock.id.clone()));
        }
        self.mocks.insert(mock.id.clone(), mock);
        Ok(())
    }

    pub fn remove_mock(&mut self, id: &str) -> Result<MockBuilder, TestHarnessError> {
        self.mocks
            .remove(id)
            .ok_or_else(|| TestHarnessError::MockNotFound(id.to_string()))
    }

    pub fn invoke_mock(&mut self, id: &str) -> Result<String, TestHarnessError> {
        let mock = self
            .mocks
            .get_mut(id)
            .ok_or_else(|| TestHarnessError::MockNotFound(id.to_string()))?;

        if let Some(max) = mock.max_calls {
            if mock.call_count >= max {
                return Err(TestHarnessError::MockNotFound(format!(
                    "{} exceeded max calls ({})",
                    id, max
                )));
            }
        }

        let result = match &mock.behavior {
            MockBehavior::ReturnValue(v) => Ok(v.clone()),
            MockBehavior::ReturnError(e) => {
                Err(TestHarnessError::AssertionFailed(e.clone()))
            }
            MockBehavior::Delay(ms) => Ok(format!("delayed:{}", ms)),
            MockBehavior::Sequence(seq) => {
                let idx = (mock.call_count as usize) % seq.len();
                Ok(seq[idx].clone())
            }
        };

        mock.call_count += 1;
        result
    }

    // -- cases --------------------------------------------------------------

    pub fn add_case(&mut self, case: TestCase) -> Result<(), TestHarnessError> {
        if self.cases.contains_key(&case.id) {
            return Err(TestHarnessError::DuplicateFixture(case.id.clone()));
        }
        self.cases.insert(case.id.clone(), case);
        Ok(())
    }

    pub fn run_case(&mut self, id: &str) -> Result<TestResult2, TestHarnessError> {
        let case = self
            .cases
            .get(id)
            .ok_or_else(|| TestHarnessError::MockNotFound(id.to_string()))?;

        // Validate referenced fixtures exist
        for fid in &case.fixture_ids {
            if !self.fixtures.contains_key(fid) {
                return Err(TestHarnessError::FixtureNotFound(fid.clone()));
            }
        }

        // Validate referenced mocks exist
        for mid in &case.mock_ids {
            if !self.mocks.contains_key(mid) {
                return Err(TestHarnessError::MockNotFound(mid.clone()));
            }
        }

        let now = Utc::now().to_rfc3339();
        let result = TestResult2::Pass;

        let case = self.cases.get_mut(id).unwrap();
        case.result = Some(result.clone());
        case.executed_at = Some(now);
        case.duration_ms = Some(0);

        Ok(result)
    }

    // -- suites -------------------------------------------------------------

    pub fn add_suite(&mut self, suite: TestSuite2) -> Result<(), TestHarnessError> {
        if self.suites.contains_key(&suite.id) {
            return Err(TestHarnessError::DuplicateFixture(suite.id.clone()));
        }
        self.suites.insert(suite.id.clone(), suite);
        Ok(())
    }

    pub fn run_suite(&mut self, id: &str) -> Result<&TestSuite2, TestHarnessError> {
        let suite = self
            .suites
            .get(id)
            .ok_or_else(|| TestHarnessError::MockNotFound(id.to_string()))?;

        let case_ids: Vec<String> = suite.cases.clone();
        let mut passed: u32 = 0;
        let mut failed: u32 = 0;
        let mut skipped: u32 = 0;
        let mut errors: u32 = 0;

        for cid in &case_ids {
            match self.run_case(cid) {
                Ok(TestResult2::Pass) => passed += 1,
                Ok(TestResult2::Fail(_)) => failed += 1,
                Ok(TestResult2::Skip(_)) => skipped += 1,
                Ok(TestResult2::Error(_)) => errors += 1,
                Err(_) => errors += 1,
            }
        }

        let suite = self.suites.get_mut(id).unwrap();
        suite.passed = passed;
        suite.failed = failed;
        suite.skipped = skipped;
        suite.errors = errors;
        suite.executed_at = Some(Utc::now().to_rfc3339());

        Ok(self.suites.get(id).unwrap())
    }

    // -- assertions ---------------------------------------------------------

    pub fn assert_eq_check(
        &self,
        expected: &str,
        actual: &str,
    ) -> Result<(), TestHarnessError> {
        if expected == actual {
            Ok(())
        } else {
            Err(TestHarnessError::AssertionFailed(format!(
                "expected '{}', got '{}'",
                expected, actual
            )))
        }
    }

    // -- search -------------------------------------------------------------

    pub fn search_fixtures(&self, query: &str) -> Vec<&TestFixture> {
        let q = query.to_lowercase();
        self.fixtures
            .values()
            .filter(|f| {
                f.name.to_lowercase().contains(&q)
                    || f.id.to_lowercase().contains(&q)
                    || f.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }

    // -- stats --------------------------------------------------------------

    pub fn stats(&self) -> HarnessStats {
        let total_cases = self.cases.len();
        let executed: Vec<&TestCase> = self
            .cases
            .values()
            .filter(|c| c.result.is_some())
            .collect();
        let passed = executed
            .iter()
            .filter(|c| c.result == Some(TestResult2::Pass))
            .count();
        let total_executions = executed.len() as u64;
        let pass_rate = if total_executions > 0 {
            passed as f64 / total_executions as f64
        } else {
            0.0
        };

        HarnessStats {
            total_fixtures: self.fixtures.len(),
            total_mocks: self.mocks.len(),
            total_cases,
            total_suites: self.suites.len(),
            pass_rate,
            total_executions,
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), TestHarnessError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, TestHarnessError> {
        let data = std::fs::read_to_string(path)?;
        let harness: Self = serde_json::from_str(&data)?;
        Ok(harness)
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
    use std::env::temp_dir;
    use std::process::id;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("evaporchain_test_{}_{}", id(), name))
    }

    fn sample_fixture(id_str: &str, ft: FixtureType, tags: Vec<&str>) -> TestFixture {
        TestFixture {
            id: id_str.to_string(),
            fixture_type: ft,
            name: format!("fixture_{}", id_str),
            data: HashMap::new(),
            created_at: Utc::now().to_rfc3339(),
            tags: tags.into_iter().map(String::from).collect(),
        }
    }

    fn sample_mock(id_str: &str, behavior: MockBehavior) -> MockBuilder {
        MockBuilder {
            id: id_str.to_string(),
            name: format!("mock_{}", id_str),
            behavior,
            call_count: 0,
            max_calls: None,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn sample_case(
        id_str: &str,
        fixture_ids: Vec<&str>,
        mock_ids: Vec<&str>,
    ) -> TestCase {
        TestCase {
            id: id_str.to_string(),
            name: format!("case_{}", id_str),
            description: "A test case".to_string(),
            fixture_ids: fixture_ids.into_iter().map(String::from).collect(),
            mock_ids: mock_ids.into_iter().map(String::from).collect(),
            result: None,
            executed_at: None,
            duration_ms: None,
        }
    }

    fn sample_suite(id_str: &str, case_ids: Vec<&str>) -> TestSuite2 {
        TestSuite2 {
            id: id_str.to_string(),
            name: format!("suite_{}", id_str),
            cases: case_ids.into_iter().map(String::from).collect(),
            passed: 0,
            failed: 0,
            skipped: 0,
            errors: 0,
            executed_at: None,
        }
    }

    #[test]
    fn test_add_fixture() {
        let mut h = TestHarness::new();
        let f = sample_fixture("f1", FixtureType::Account, vec!["defi"]);
        assert!(h.add_fixture(f).is_ok());
        assert!(h.get_fixture("f1").is_some());
    }

    #[test]
    fn test_remove_fixture() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("f1", FixtureType::Account, vec![])).unwrap();
        let removed = h.remove_fixture("f1").unwrap();
        assert_eq!(removed.id, "f1");
        assert!(h.get_fixture("f1").is_none());
    }

    #[test]
    fn test_duplicate_fixture() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("f1", FixtureType::Account, vec![])).unwrap();
        let res = h.add_fixture(sample_fixture("f1", FixtureType::Token, vec![]));
        assert!(res.is_err());
    }

    #[test]
    fn test_fixture_not_found() {
        let mut h = TestHarness::new();
        assert!(h.remove_fixture("nonexistent").is_err());
    }

    #[test]
    fn test_fixtures_by_type() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("f1", FixtureType::Account, vec![])).unwrap();
        h.add_fixture(sample_fixture("f2", FixtureType::Token, vec![])).unwrap();
        h.add_fixture(sample_fixture("f3", FixtureType::Account, vec![])).unwrap();
        let accounts = h.fixtures_by_type(&FixtureType::Account);
        assert_eq!(accounts.len(), 2);
    }

    #[test]
    fn test_fixtures_by_tag() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("f1", FixtureType::Account, vec!["defi", "v2"])).unwrap();
        h.add_fixture(sample_fixture("f2", FixtureType::Token, vec!["defi"])).unwrap();
        h.add_fixture(sample_fixture("f3", FixtureType::Block, vec!["v2"])).unwrap();
        let defi = h.fixtures_by_tag("defi");
        assert_eq!(defi.len(), 2);
    }

    #[test]
    fn test_create_mock() {
        let mut h = TestHarness::new();
        let m = sample_mock("m1", MockBehavior::ReturnValue("ok".into()));
        assert!(h.create_mock(m).is_ok());
    }

    #[test]
    fn test_remove_mock() {
        let mut h = TestHarness::new();
        h.create_mock(sample_mock("m1", MockBehavior::ReturnValue("ok".into()))).unwrap();
        let removed = h.remove_mock("m1").unwrap();
        assert_eq!(removed.id, "m1");
    }

    #[test]
    fn test_invoke_mock_return_value() {
        let mut h = TestHarness::new();
        h.create_mock(sample_mock("m1", MockBehavior::ReturnValue("hello".into()))).unwrap();
        let val = h.invoke_mock("m1").unwrap();
        assert_eq!(val, "hello");
    }

    #[test]
    fn test_invoke_mock_return_error() {
        let mut h = TestHarness::new();
        h.create_mock(sample_mock("m1", MockBehavior::ReturnError("boom".into()))).unwrap();
        let res = h.invoke_mock("m1");
        assert!(res.is_err());
    }

    #[test]
    fn test_invoke_mock_delay() {
        let mut h = TestHarness::new();
        h.create_mock(sample_mock("m1", MockBehavior::Delay(500))).unwrap();
        let val = h.invoke_mock("m1").unwrap();
        assert_eq!(val, "delayed:500");
    }

    #[test]
    fn test_invoke_mock_sequence() {
        let mut h = TestHarness::new();
        h.create_mock(sample_mock(
            "m1",
            MockBehavior::Sequence(vec!["a".into(), "b".into(), "c".into()]),
        ))
        .unwrap();
        assert_eq!(h.invoke_mock("m1").unwrap(), "a");
        assert_eq!(h.invoke_mock("m1").unwrap(), "b");
        assert_eq!(h.invoke_mock("m1").unwrap(), "c");
        assert_eq!(h.invoke_mock("m1").unwrap(), "a"); // wraps around
    }

    #[test]
    fn test_invoke_mock_max_calls() {
        let mut h = TestHarness::new();
        let mut m = sample_mock("m1", MockBehavior::ReturnValue("ok".into()));
        m.max_calls = Some(2);
        h.create_mock(m).unwrap();
        assert!(h.invoke_mock("m1").is_ok());
        assert!(h.invoke_mock("m1").is_ok());
        assert!(h.invoke_mock("m1").is_err()); // exceeded
    }

    #[test]
    fn test_invoke_mock_not_found() {
        let mut h = TestHarness::new();
        assert!(h.invoke_mock("nonexistent").is_err());
    }

    #[test]
    fn test_add_and_run_case() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("f1", FixtureType::Account, vec![])).unwrap();
        h.create_mock(sample_mock("m1", MockBehavior::ReturnValue("ok".into()))).unwrap();
        h.add_case(sample_case("c1", vec!["f1"], vec!["m1"])).unwrap();
        let result = h.run_case("c1").unwrap();
        assert_eq!(result, TestResult2::Pass);
        let case = h.cases.get("c1").unwrap();
        assert!(case.executed_at.is_some());
        assert!(case.duration_ms.is_some());
    }

    #[test]
    fn test_run_case_missing_fixture() {
        let mut h = TestHarness::new();
        h.add_case(sample_case("c1", vec!["missing"], vec![])).unwrap();
        assert!(h.run_case("c1").is_err());
    }

    #[test]
    fn test_run_case_missing_mock() {
        let mut h = TestHarness::new();
        h.add_case(sample_case("c1", vec![], vec!["missing"])).unwrap();
        assert!(h.run_case("c1").is_err());
    }

    #[test]
    fn test_add_and_run_suite() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("f1", FixtureType::Account, vec![])).unwrap();
        h.add_case(sample_case("c1", vec!["f1"], vec![])).unwrap();
        h.add_case(sample_case("c2", vec!["f1"], vec![])).unwrap();
        h.add_suite(sample_suite("s1", vec!["c1", "c2"])).unwrap();
        let suite = h.run_suite("s1").unwrap();
        assert_eq!(suite.passed, 2);
        assert_eq!(suite.failed, 0);
        assert!(suite.executed_at.is_some());
    }

    #[test]
    fn test_assert_eq_pass() {
        let h = TestHarness::new();
        assert!(h.assert_eq_check("hello", "hello").is_ok());
    }

    #[test]
    fn test_assert_eq_fail() {
        let h = TestHarness::new();
        let res = h.assert_eq_check("hello", "world");
        assert!(res.is_err());
    }

    #[test]
    fn test_search_fixtures() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("alpha", FixtureType::Account, vec!["swap"])).unwrap();
        h.add_fixture(sample_fixture("beta", FixtureType::Token, vec!["bridge"])).unwrap();
        let results = h.search_fixtures("alpha");
        assert_eq!(results.len(), 1);
        let results = h.search_fixtures("bridge");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_stats() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("f1", FixtureType::Account, vec![])).unwrap();
        h.create_mock(sample_mock("m1", MockBehavior::ReturnValue("ok".into()))).unwrap();
        h.add_case(sample_case("c1", vec!["f1"], vec!["m1"])).unwrap();
        h.add_suite(sample_suite("s1", vec!["c1"])).unwrap();
        h.run_case("c1").unwrap();

        let stats = h.stats();
        assert_eq!(stats.total_fixtures, 1);
        assert_eq!(stats.total_mocks, 1);
        assert_eq!(stats.total_cases, 1);
        assert_eq!(stats.total_suites, 1);
        assert!((stats.pass_rate - 1.0).abs() < f64::EPSILON);
        assert_eq!(stats.total_executions, 1);
    }

    #[test]
    fn test_save_and_load() {
        let mut h = TestHarness::new();
        h.add_fixture(sample_fixture("f1", FixtureType::Account, vec!["tag1"])).unwrap();
        h.create_mock(sample_mock("m1", MockBehavior::ReturnValue("v".into()))).unwrap();

        let path = tmp_path("harness.json");
        h.save(&path).unwrap();

        let loaded = TestHarness::load(&path).unwrap();
        assert_eq!(loaded.fixtures.len(), 1);
        assert_eq!(loaded.mocks.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default() {
        let path = tmp_path("nonexistent_harness.json");
        let h = TestHarness::load_or_default(&path);
        assert!(h.fixtures.is_empty());
        assert!(h.mocks.is_empty());
    }
}
