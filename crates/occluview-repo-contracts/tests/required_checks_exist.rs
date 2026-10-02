//! Keeps the required status names aligned with jobs that run on pull requests.

#![allow(clippy::expect_used, clippy::panic)] // missing contract inputs fail the test

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate manifest is inside the workspace")
        .to_path_buf()
}

fn indentation(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn workflow_runs_on_pull_request(workflow: &str) -> bool {
    for line in workflow.lines() {
        let trimmed = line.trim();
        if indentation(line) == 0 && trimmed == "jobs:" {
            break;
        }
        if indentation(line) == 2 && trimmed.starts_with("pull_request:") {
            return true;
        }
    }
    false
}

fn scalar_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let value = line.trim().strip_prefix(key)?.trim();
    if let Some(value) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return Some(value);
    }
    if let Some(value) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return Some(value);
    }
    Some(value)
}

fn pull_request_job_names(workflow: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut in_jobs = false;
    let mut job_name = None;
    let mut schedule_only = false;

    for line in workflow.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line_indent = indentation(line);
        if line_indent == 0 && trimmed == "jobs:" {
            in_jobs = true;
            continue;
        }
        if !in_jobs {
            continue;
        }
        if line_indent == 0 {
            break;
        }
        if line_indent == 2 && trimmed.ends_with(':') {
            if !schedule_only {
                if let Some(name) = job_name.take() {
                    names.insert(name);
                }
            }
            job_name = None;
            schedule_only = false;
            continue;
        }
        if line_indent == 4 {
            if let Some(name) = scalar_value(line, "name:") {
                job_name = Some(name.to_owned());
            } else if scalar_value(line, "if:") == Some("github.event_name == 'schedule'") {
                schedule_only = true;
            }
        }
    }

    if !schedule_only {
        if let Some(name) = job_name {
            names.insert(name);
        }
    }
    names
}

#[test]
fn required_checks_match_pull_request_jobs() {
    let root = repository_root();
    let required_text = fs::read_to_string(root.join(".github/required-checks.txt"))
        .expect("required check list is readable");
    let required: BTreeSet<String> = required_text
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    assert!(!required.is_empty(), "required check list is not empty");

    let workflows_dir = root.join(".github/workflows");
    let mut pull_request_jobs = BTreeSet::new();
    for entry in fs::read_dir(&workflows_dir).expect("workflow directory is readable") {
        let path = entry.expect("workflow entry is readable").path();
        if !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let workflow = fs::read_to_string(path).expect("workflow file is readable");
        if workflow_runs_on_pull_request(&workflow) {
            pull_request_jobs.extend(pull_request_job_names(&workflow));
        }
    }

    assert_eq!(
        required, pull_request_jobs,
        "the required check list matches explicitly named jobs that run on pull requests"
    );
}
