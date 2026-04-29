use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LadScriptError {
    #[error("bad @lad annotation: {0}")]
    BadAnnotation(String),
    #[error("unknown LAD resource: {0:?}")]
    UnknownResource(String),
    #[error("LAD-VM error on resource {name:?}: {detail}")]
    LadVmError { name: String, detail: String },
    #[error("script execution failed: {0}")]
    ScriptExecution(String),
}
