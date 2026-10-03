//! The manifest fields crates.io checks when it receives a crate and
//! `cargo publish --dry-run` does not: a crate that fails here is refused by
//! the registry after the release has started. Part of `cargo xtask check`.
//!
//! The rules follow crates.io's publish endpoint
//! (`src/controllers/krate/publish.rs` in rust-lang/crates.io):
//! - `description` present, at most 1000 characters;
//! - `license` present, an SPDX expression of the licences listed below;
//! - `homepage`, `documentation` and `repository` start with `http://` or
//!   `https://` and hold no whitespace;
//! - at most 5 keywords, each at most 20 bytes, starting with an ASCII letter
//!   or digit and made of ASCII letters, digits, `-`, `_` and `+`;
//! - at most 5 categories, each a slug crates.io knows
//!   (`xtask/crates-io-categories.txt`; an unknown one would be dropped with
//!   only a warning);
//! - at most 300 features;
//! - a `readme` that exists.
//!
//! The size limit (10 MB) is the package check's (`MAX_PACKAGE_BYTES`).

use std::path::Path;

use crate::util::{Result, cargo, output, root};

const MAX_DESCRIPTION: usize = 1000;
const MAX_KEYWORDS: usize = 5;
const MAX_KEYWORD_BYTES: usize = 20;
const MAX_CATEGORIES: usize = 5;
const MAX_FEATURES: usize = 300;

/// The SPDX identifiers a licence expression may use here. Another licence is
/// a decision, and belongs in this list first.
const LICENSES: [&str; 2] = ["MIT", "Apache-2.0"];

/// The list of category slugs, relative to the workspace root.
const CATEGORIES_FILE: &str = "xtask/crates-io-categories.txt";

pub(crate) fn check() -> Result<()> {
    let text = output(cargo().args(["metadata", "--no-deps", "--format-version", "1"]))?;
    let metadata: serde_json::Value = serde_json::from_str(&text)?;
    let categories = std::fs::read_to_string(root().join(CATEGORIES_FILE))?;
    let known: Vec<&str> = categories
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let mut problems = Vec::new();
    let mut checked = 0;
    for package in metadata["packages"].as_array().into_iter().flatten() {
        // `publish = false` is an empty list; any other value publishes.
        if package["publish"].as_array().is_some_and(Vec::is_empty) {
            continue;
        }
        checked += 1;
        let name = package["name"].as_str().unwrap_or("?");
        for problem in package_problems(package, &known) {
            problems.push(format!("{name}: {problem}"));
        }
    }
    if !problems.is_empty() {
        return Err(format!(
            "crates.io would refuse or change these manifests:\n  {}",
            problems.join("\n  ")
        )
        .into());
    }
    eprintln!("check: crates.io metadata rules hold for {checked} crates");
    Ok(())
}

fn package_problems(package: &serde_json::Value, known: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    let text = |key: &str| package[key].as_str().filter(|s| !s.trim().is_empty());
    let list = |key: &str| -> Vec<&str> {
        package[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect()
    };

    match text("description") {
        None => problems.push("no description".to_owned()),
        Some(d) if d.chars().count() > MAX_DESCRIPTION => {
            problems.push(format!(
                "the description is longer than {MAX_DESCRIPTION} characters"
            ));
        }
        Some(_) => {}
    }
    match text("license") {
        None => problems.push("no license".to_owned()),
        Some(l) => {
            if let Some(problem) = license_problem(l) {
                problems.push(problem);
            }
        }
    }
    for key in ["homepage", "documentation", "repository"] {
        if let Some(problem) = text(key).and_then(|url| url_problem(key, url)) {
            problems.push(problem);
        }
    }
    problems.extend(keyword_problems(&list("keywords")));
    problems.extend(category_problems(&list("categories"), known));
    let features = package["features"]
        .as_object()
        .map_or(0, serde_json::Map::len);
    if features > MAX_FEATURES {
        problems.push(format!("{features} features, more than {MAX_FEATURES}"));
    }
    match (text("readme"), text("manifest_path")) {
        (None, _) => problems.push("no readme".to_owned()),
        (Some(readme), Some(manifest)) => {
            let dir = Path::new(manifest).parent().unwrap_or(Path::new("."));
            if !dir.join(readme).is_file() {
                problems.push(format!("the readme {readme} does not exist"));
            }
        }
        (Some(_), None) => {}
    }
    problems
}

/// crates.io's keyword rules.
fn keyword_problems(keywords: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    if keywords.len() > MAX_KEYWORDS {
        problems.push(format!(
            "{} keywords, more than {MAX_KEYWORDS}",
            keywords.len()
        ));
    }
    for keyword in keywords {
        if keyword.len() > MAX_KEYWORD_BYTES {
            problems.push(format!(
                "the keyword {keyword:?} is longer than {MAX_KEYWORD_BYTES} characters"
            ));
            continue;
        }
        let mut chars = keyword.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+'));
        if !valid {
            problems.push(format!(
                "the keyword {keyword:?} is not an ASCII letter or digit followed by letters, \
                 digits, `-`, `_` or `+`"
            ));
        }
    }
    problems
}

fn category_problems(categories: &[&str], known: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    if categories.len() > MAX_CATEGORIES {
        problems.push(format!(
            "{} categories, more than {MAX_CATEGORIES}",
            categories.len()
        ));
    }
    for category in categories {
        if !known.contains(category) {
            problems.push(format!(
                "the category {category:?} is not in {CATEGORIES_FILE}"
            ));
        }
    }
    problems
}

fn url_problem(key: &str, url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(rest)
            if !rest.is_empty() && !rest.starts_with('/') && !url.contains(char::is_whitespace) =>
        {
            None
        }
        _ => Some(format!("{key} {url:?} is not an http:// or https:// URL")),
    }
}

/// The expression may combine the identifiers of [`LICENSES`] with `OR`,
/// `AND` and parentheses.
fn license_problem(expression: &str) -> Option<String> {
    let spaced = expression.replace('(', " ( ").replace(')', " ) ");
    let unknown: Vec<&str> = spaced
        .split_whitespace()
        .filter(|t| !matches!(*t, "OR" | "AND" | "(" | ")") && !LICENSES.contains(t))
        .collect();
    if unknown.is_empty() {
        None
    } else {
        Some(format!(
            "the license expression {expression:?} uses {}, not one of {}",
            unknown.join(", "),
            LICENSES.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_follow_the_registry() {
        assert_eq!(
            keyword_problems(&["clingo", "answer-set", "asp", "c++", "ffi_2"]),
            Vec::<String>::new()
        );
        // 22 bytes: the keyword crates.io refused.
        assert_eq!(keyword_problems(&["answer-set-programming"]).len(), 1);
        assert_eq!(keyword_problems(&["-asp"]).len(), 1);
        assert_eq!(keyword_problems(&["answer set"]).len(), 1);
        assert_eq!(keyword_problems(&["a", "b", "c", "d", "e", "f"]).len(), 1);
        assert_eq!(
            keyword_problems(&["exactly-twenty-chars"]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn categories_must_be_known_slugs() {
        let known = ["api-bindings", "science"];
        assert_eq!(
            category_problems(&["science"], &known),
            Vec::<String>::new()
        );
        assert_eq!(
            category_problems(&["answer-set-programming"], &known).len(),
            1
        );
    }

    #[test]
    fn urls_and_licenses() {
        assert!(url_problem("homepage", "https://example.org/x").is_none());
        assert!(url_problem("homepage", "example.org").is_some());
        assert!(url_problem("homepage", "https:///x").is_some());
        assert!(license_problem("MIT OR Apache-2.0").is_none());
        assert!(license_problem("(MIT OR Apache-2.0)").is_none());
        assert!(license_problem("GPL-3.0").is_some());
    }
}
