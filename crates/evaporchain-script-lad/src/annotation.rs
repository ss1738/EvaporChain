//! Parses `@lad(...)` annotation lines from EvaporScript source.
//!
//! Annotation syntax (one per line, before the field/variable declaration):
//!
//! ```text
//! @lad(mode=linear)
//! @lad(mode=affine)
//! @lad(mode=decaying, window=50)
//! ```
//!
//! The parser is line-oriented and intentionally forgiving — unknown keys are
//! ignored so annotations survive future additions without breaking older scripts.

use crate::error::LadScriptError;
use evaporchain_lad_vm::Mode;

/// A parsed `@lad(...)` annotation attached to one script variable/field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LadAnnotation {
    /// Name of the variable or state field this annotation applies to.
    pub field_name: String,
    pub mode: Mode,
    /// Decay window in epochs (only meaningful for `Mode::Decaying`).
    pub decay_window: Option<u64>,
    /// Initial value declared at annotation site (0 if not specified).
    pub initial_value: u64,
}

/// Scan `source` for `@lad(...)` annotations and return them in declaration order.
///
/// Convention: the annotation must appear on the line *immediately* before the
/// `let <name>` declaration. Lines without a following `let` are silently skipped.
///
/// Example:
/// ```text
/// @lad(mode=decaying, window=100, value=1000)
/// let my_resource: u64 = 0;
/// ```
pub fn parse_annotations(source: &str) -> Result<Vec<LadAnnotation>, LadScriptError> {
    let mut annotations = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if let Some(inner) = trimmed.strip_prefix("@lad(").and_then(|s| s.strip_suffix(')')) {
            // Look ahead for the field name on the next non-blank line.
            let field_name = find_next_let_name(&lines, i + 1);
            if let Some(name) = field_name {
                let ann = parse_inner(inner, name)?;
                annotations.push(ann);
            }
        }
        i += 1;
    }
    Ok(annotations)
}

fn find_next_let_name(lines: &[&str], start: usize) -> Option<String> {
    for line in lines.iter().skip(start) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Match `let <name>` or `let mut <name>`
        if let Some(rest) = t.strip_prefix("let ") {
            let rest = rest.trim_start_matches("mut").trim();
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
        break; // Non-blank non-let line: no match.
    }
    None
}

fn parse_inner(inner: &str, field_name: String) -> Result<LadAnnotation, LadScriptError> {
    let mut mode_str: Option<&str> = None;
    let mut decay_window: Option<u64> = None;
    let mut initial_value: u64 = 0;

    for part in inner.split(',') {
        let kv: Vec<&str> = part.splitn(2, '=').map(|s| s.trim()).collect();
        match kv.as_slice() {
            ["mode", v] => mode_str = Some(v),
            ["window", v] => {
                decay_window = Some(v.parse::<u64>().map_err(|_| {
                    LadScriptError::BadAnnotation(format!("window must be u64, got {v:?}"))
                })?)
            }
            ["value", v] => {
                initial_value = v.parse::<u64>().map_err(|_| {
                    LadScriptError::BadAnnotation(format!("value must be u64, got {v:?}"))
                })?
            }
            _ => {} // Unknown keys silently ignored.
        }
    }

    let mode = match mode_str {
        Some("linear") => Mode::Linear,
        Some("affine") => Mode::Affine,
        Some("decaying") => Mode::Decaying,
        Some(other) => {
            return Err(LadScriptError::BadAnnotation(format!(
                "unknown lad mode {other:?}; expected linear|affine|decaying"
            )))
        }
        None => return Err(LadScriptError::BadAnnotation("@lad annotation missing mode=".into())),
    };

    Ok(LadAnnotation {
        field_name,
        mode,
        decay_window,
        initial_value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_linear() {
        let src = "@lad(mode=linear, value=500)\nlet my_token: u64 = 0;";
        let anns = parse_annotations(src).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].field_name, "my_token");
        assert_eq!(anns[0].mode, Mode::Linear);
        assert_eq!(anns[0].initial_value, 500);
        assert_eq!(anns[0].decay_window, None);
    }

    #[test]
    fn parse_decaying_with_window() {
        let src = "@lad(mode=decaying, window=100, value=1000)\nlet voucher: u64 = 0;";
        let anns = parse_annotations(src).unwrap();
        assert_eq!(anns[0].mode, Mode::Decaying);
        assert_eq!(anns[0].decay_window, Some(100));
        assert_eq!(anns[0].initial_value, 1000);
    }

    #[test]
    fn parse_multiple_annotations() {
        let src = "\
@lad(mode=linear, value=100)\n\
let tok_a: u64 = 0;\n\
@lad(mode=affine, value=200)\n\
let tok_b: u64 = 0;";
        let anns = parse_annotations(src).unwrap();
        assert_eq!(anns.len(), 2);
        assert_eq!(anns[0].field_name, "tok_a");
        assert_eq!(anns[1].field_name, "tok_b");
    }

    #[test]
    fn annotation_without_following_let_silently_skipped() {
        let src = "@lad(mode=linear, value=50)\n// just a comment";
        let anns = parse_annotations(src).unwrap();
        assert!(anns.is_empty());
    }

    #[test]
    fn unknown_key_ignored() {
        let src = "@lad(mode=linear, value=50, future_key=xyz)\nlet x: u64 = 0;";
        let anns = parse_annotations(src).unwrap();
        assert_eq!(anns[0].field_name, "x");
    }

    #[test]
    fn bad_mode_returns_error() {
        let src = "@lad(mode=quantum)\nlet x: u64 = 0;";
        assert!(parse_annotations(src).is_err());
    }
}
