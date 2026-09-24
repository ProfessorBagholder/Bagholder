//! Every plan checks itself against how the leading products do it
//! (`PLAN.template.md`, "How the leading products do it"; owner, 2026-09-24):
//! a plan in `docs/plans/` without that section, or without a source link in it,
//! fails here, so the check never rests on anyone remembering it.

use std::path::Path;

const SECTION: &str = "## How the leading products do it";

/// Written before the section existed, each already through the gate; nothing
/// is added to this list.
const BEFORE_THE_RULE: &[&str] = &["stage-1-foundation.md", "stage-2-engine.md", "stage-3a-sources.md", "stage-3a-brief-06.md"];

/// Evidence and reference notes that are not plans.
const NOT_PLANS: &[&str] = &["stage-3a-research.md", "time-zone-rules.md"];

#[test]
fn every_plan_says_how_the_leading_products_do_it_with_its_sources() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/plans");
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".md") || BEFORE_THE_RULE.contains(&name.as_str()) || NOT_PLANS.contains(&name.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let Some(at) = text.find(SECTION) else {
            failures.push(format!("{name}: no \"{SECTION}\" section"));
            continue;
        };
        let body = &text[at + SECTION.len()..];
        let section = body.find("\n## ").map_or(body, |end| &body[..end]);
        if !section.contains("https://") {
            failures.push(format!("{name}: \"{SECTION}\" cites no source link"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_list_of_plans_before_the_rule_names_only_files_that_exist() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/plans");
    for name in BEFORE_THE_RULE.iter().chain(NOT_PLANS) {
        assert!(dir.join(name).exists(), "{name} is listed but does not exist");
    }
}
