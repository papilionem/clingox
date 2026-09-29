//! `docs/dev/COVERAGE.md`: one row per function and global in `clingo.h`.
//!
//! The generated columns (group, return type, meaning of a `bool` return) come
//! from the header. The Wrapper, Test and Notes columns are written by hand as
//! functions are wrapped (RULES 3), so regeneration keeps them.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::bindings::header;
use crate::util::{Result, root};

/// The number of functions in the clingo 5.8.2 header (DESIGN 8.1).
const EXPECTED_FUNCTIONS: usize = 254;
const EXPECTED_DATA: usize = 2;

struct Item {
    group: String,
    name: String,
    /// The C return type for functions, the declared type for globals.
    ty: String,
    /// What a `bool` return means, read from the `@return` line of the docs.
    bool_meaning: &'static str,
}

#[derive(Default, Clone)]
struct Manual {
    wrapper: String,
    test: String,
    notes: String,
}

pub(crate) fn run() -> Result<()> {
    let text = std::fs::read_to_string(header())?;
    let (functions, data) = parse_header(&text)?;
    if functions.len() != EXPECTED_FUNCTIONS || data.len() != EXPECTED_DATA {
        return Err(format!(
            "clingo.h declares {} functions and {} globals, expected {EXPECTED_FUNCTIONS} and \
             {EXPECTED_DATA}; update the expectation when bumping clingo (UPGRADING.md)",
            functions.len(),
            data.len()
        )
        .into());
    }

    let path = output();
    let manual = std::fs::read_to_string(&path)
        .map(|old| read_manual_columns(&old))
        .unwrap_or_default();
    check_listed_tests(&manual)?;
    let dropped: Vec<&String> = manual
        .keys()
        .filter(|name| {
            !functions
                .iter()
                .chain(&data)
                .any(|item| &item.name == *name)
        })
        .collect();
    if !dropped.is_empty() {
        eprintln!("coverage: rows no longer in clingo.h were removed: {dropped:?}");
    }

    std::fs::write(&path, render(&functions, &data, &manual))?;
    eprintln!(
        "coverage: wrote {} with {} function rows and {} data rows",
        path.display(),
        functions.len(),
        data.len()
    );
    Ok(())
}

fn output() -> PathBuf {
    root().join("docs/dev/COVERAGE.md")
}

/// Walks the header's Doxygen groups and collects every exported declaration.
fn parse_header(text: &str) -> Result<(Vec<Item>, Vec<Item>)> {
    let mut functions = Vec::new();
    let mut data = Vec::new();
    let mut group = String::from("(none)");
    let mut doc = String::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("//!") {
            let rest = rest.trim();
            if let Some(name) = rest
                .strip_prefix("@addtogroup ")
                .or_else(|| rest.strip_prefix("@defgroup "))
            {
                name.split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .clone_into(&mut group);
            }
            doc.push_str(rest);
            doc.push('\n');
            continue;
        }
        let Some(decl) = trimmed.strip_prefix("CLINGO_VISIBILITY_DEFAULT ") else {
            doc.clear();
            continue;
        };
        let mut decl = decl.to_owned();
        while !decl.contains(';') {
            let next = lines.next().ok_or("unterminated declaration in clingo.h")?;
            decl.push(' ');
            decl.push_str(next.trim());
        }
        if let Some(global) = decl.strip_prefix("extern ") {
            let global = global.trim_end_matches(';').trim();
            let (ty, name) = global
                .rsplit_once(' ')
                .ok_or("malformed global in clingo.h")?;
            data.push(Item {
                group: group.clone(),
                name: name.to_owned(),
                ty: ty.to_owned(),
                bool_meaning: "",
            });
        } else {
            let head = decl.split('(').next().unwrap_or_default().trim();
            let (ty, name) = head
                .rsplit_once(' ')
                .ok_or("malformed function in clingo.h")?;
            let bool_meaning = if ty != "bool" {
                ""
            } else if doc.contains("@return whether the call was successful")
                || doc.contains("@return whether the function call was successful")
            {
                "success flag"
            } else {
                "value"
            };
            functions.push(Item {
                group: group.clone(),
                name: name.trim_start_matches('*').to_owned(),
                ty: ty.to_owned(),
                bool_meaning,
            });
        }
        doc.clear();
    }
    Ok((functions, data))
}

/// Reads the hand-written columns of an existing COVERAGE.md, keyed by C name.
fn read_manual_columns(old: &str) -> HashMap<String, Manual> {
    let mut rows = HashMap::new();
    for line in old.lines() {
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let Some(name) = cells
            .first()
            .and_then(|c| c.strip_prefix('`')?.strip_suffix('`'))
        else {
            continue;
        };
        if !name.starts_with("clingo_") && !name.starts_with("g_clingo_") {
            continue;
        }
        // Function rows have 6 cells and data rows 5; the last three are manual.
        let n = cells.len();
        if n >= 5 {
            rows.insert(
                name.to_owned(),
                Manual {
                    wrapper: cells[n - 3].to_owned(),
                    test: cells[n - 2].to_owned(),
                    notes: cells[n - 1].to_owned(),
                },
            );
        }
    }
    rows
}

/// A listed test must exist, so a row cannot claim coverage that was deleted
/// (TESTING 8). The Test cell holds `path/to/file.rs` or `path/to/file.rs::name`.
fn check_listed_tests(manual: &HashMap<String, Manual>) -> Result<()> {
    let mut missing = Vec::new();
    for (name, row) in manual {
        let test = row.test.trim_matches('`');
        if test.is_empty() {
            continue;
        }
        let file = test.split("::").next().unwrap_or(test);
        if !root().join(file).exists() {
            missing.push(format!("{name}: {test}"));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        missing.sort();
        Err(format!(
            "COVERAGE.md lists tests that do not exist:\n{}",
            missing.join("\n")
        )
        .into())
    }
}

fn render(functions: &[Item], data: &[Item], manual: &HashMap<String, Manual>) -> String {
    let mut groups: Vec<&str> = Vec::new();
    for item in functions {
        if !groups.contains(&item.group.as_str()) {
            groups.push(&item.group);
        }
    }
    let wrapped = |item: &Item| {
        manual
            .get(&item.name)
            .is_some_and(|m| !m.wrapper.is_empty())
    };

    let mut out = String::new();
    out.push_str("# clingo C API coverage\n\n");
    out.push_str(
        "Generated by `cargo xtask coverage` from `clingox-sys/clingo/libclingo/clingo.h`. \
         Every function and global the header exports has one row. The group, return type \
         and the meaning of a `bool` return come from the header; a `bool` is a success flag \
         when its `@return` line says the call was successful, and a value otherwise. The \
         Wrapper, Test and Notes columns are filled in by hand as functions are wrapped \
         (RULES 3), and regeneration keeps them. A function is wrapped only when it has a \
         test; a row without a wrapper needs a note saying why it is not exposed.\n\n",
    );
    out.push_str("## Summary\n\n| Group | Functions | Wrapped |\n|---|---:|---:|\n");
    for group in &groups {
        let items: Vec<&Item> = functions.iter().filter(|i| i.group == *group).collect();
        let done = items.iter().filter(|i| wrapped(i)).count();
        let _ = writeln!(out, "| {group} | {} | {done} |", items.len());
    }
    let done = functions.iter().filter(|i| wrapped(i)).count();
    let _ = writeln!(
        out,
        "| **Total** | **{}** | **{done}** |\n",
        functions.len()
    );

    for group in &groups {
        let items: Vec<&Item> = functions.iter().filter(|i| i.group == *group).collect();
        let _ = writeln!(out, "## {group} ({})\n", items.len());
        out.push_str("| Function | Returns | `bool` is | Wrapper | Test | Notes |\n");
        out.push_str("|---|---|---|---|---|---|\n");
        for item in items {
            let m = manual.get(&item.name).cloned().unwrap_or_default();
            let _ = writeln!(
                out,
                "| `{}` | `{}` | {} | {} | {} | {} |",
                item.name, item.ty, item.bool_meaning, m.wrapper, m.test, m.notes
            );
        }
        out.push('\n');
    }

    let _ = writeln!(out, "## Data ({})\n", data.len());
    out.push_str("| Global | Type | Wrapper | Test | Notes |\n|---|---|---|---|---|\n");
    for item in data {
        let m = manual.get(&item.name).cloned().unwrap_or_default();
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} | {} | {} |",
            item.name, item.ty, m.wrapper, m.test, m.notes
        );
    }
    out
}
