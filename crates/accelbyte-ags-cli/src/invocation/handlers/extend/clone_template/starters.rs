//! Starter template catalogue: data types, embedded loading, and filtering.
//!
//! A starter is a git repository (+ optional sub-path) that scaffolds a new
//! Extend project. The catalogue is embedded at compile time from a JSON asset
//! file, so the CLI works fully offline without a network fetch.

use std::collections::BTreeSet;

/// One entry in the starter catalogue.
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct Starter {
    pub name: String,
    pub url: String,
    pub scenario: String,
    pub template: String,
    pub language: String,
    #[serde(default)]
    pub source_path: Option<String>,
}

/// Load the bundled starter catalogue from the embedded JSON asset.
pub(super) fn load_bundled_starters() -> Vec<Starter> {
    let raw = include_str!("starters.json");
    serde_json::from_str(raw).expect("embedded starters.json is valid")
}

/// Return the unique values for a field across the given starters, sorted.
pub(super) fn unique_scenarios(starters: &[Starter]) -> Vec<String> {
    unique_field(starters, |s| &s.scenario)
}

/// Return the unique template names across the given starters, sorted.
pub(super) fn unique_templates(starters: &[Starter]) -> Vec<String> {
    unique_field(starters, |s| &s.template)
}

/// Return the unique languages across the given starters, sorted.
pub(super) fn unique_languages(starters: &[Starter]) -> Vec<String> {
    unique_field(starters, |s| &s.language)
}

/// Filter starters by scenario (case-insensitive, whitespace-trimmed).
pub(super) fn filter_by_scenario(starters: &[Starter], scenario: &str) -> Vec<Starter> {
    let needle = scenario.trim().to_lowercase();
    starters
        .iter()
        .filter(|s| s.scenario.trim().eq_ignore_ascii_case(&needle))
        .cloned()
        .collect()
}

/// Filter starters by template name (case-insensitive, whitespace-trimmed).
pub(super) fn filter_by_template(starters: &[Starter], template: &str) -> Vec<Starter> {
    let needle = template.trim().to_lowercase();
    starters
        .iter()
        .filter(|s| s.template.trim().eq_ignore_ascii_case(&needle))
        .cloned()
        .collect()
}

/// Filter starters by language (case-insensitive, whitespace-trimmed).
pub(super) fn filter_by_language(starters: &[Starter], language: &str) -> Vec<Starter> {
    let needle = language.trim().to_lowercase();
    starters
        .iter()
        .filter(|s| s.language.trim().eq_ignore_ascii_case(&needle))
        .cloned()
        .collect()
}

/// Find a starter by exact name (case-insensitive).
pub(super) fn find_by_name(starters: &[Starter], name: &str) -> Option<Starter> {
    starters
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .cloned()
}

/// Extract unique sorted values for a string field across the starter list.
fn unique_field(starters: &[Starter], field: fn(&Starter) -> &String) -> Vec<String> {
    let set: BTreeSet<String> = starters.iter().map(|s| field(s).clone()).collect();
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_bundled_starters_is_non_empty() {
        let starters = load_bundled_starters();
        assert!(
            !starters.is_empty(),
            "bundled starters should contain entries"
        );
    }

    #[test]
    fn test_load_bundled_starters_all_have_required_fields() {
        for starter in load_bundled_starters() {
            assert!(!starter.name.is_empty(), "name must be non-empty");
            assert!(!starter.url.is_empty(), "url must be non-empty");
            assert!(!starter.scenario.is_empty(), "scenario must be non-empty");
            assert!(!starter.template.is_empty(), "template must be non-empty");
            assert!(!starter.language.is_empty(), "language must be non-empty");
        }
    }

    #[test]
    fn test_unique_scenarios_returns_sorted_deduplicated() {
        let starters = load_bundled_starters();
        let scenarios = unique_scenarios(&starters);
        assert!(scenarios.len() >= 2, "expected multiple scenarios");
        let mut sorted = scenarios.clone();
        sorted.sort();
        assert_eq!(scenarios, sorted, "scenarios must be sorted");
    }

    #[test]
    fn test_filter_by_scenario_case_insensitive() {
        let starters = load_bundled_starters();
        let filtered = filter_by_scenario(&starters, "extend override");
        assert!(
            !filtered.is_empty(),
            "should match 'Extend Override' case-insensitively"
        );
        for s in &filtered {
            assert_eq!(
                s.scenario.to_lowercase(),
                "extend override",
                "all filtered starters must match"
            );
        }
    }

    #[test]
    fn test_filter_by_template_narrows_results() {
        let starters = load_bundled_starters();
        let by_scenario = filter_by_scenario(&starters, "Extend Override");
        let by_template = filter_by_template(&by_scenario, "Lootbox Roll");
        assert!(!by_template.is_empty(), "should find Lootbox Roll starters");
        assert!(
            by_template.len() < by_scenario.len(),
            "filtering by template should narrow results"
        );
    }

    #[test]
    fn test_filter_by_language_narrows_results() {
        let starters = load_bundled_starters();
        let by_lang = filter_by_language(&starters, "Go");
        assert!(!by_lang.is_empty(), "should find Go starters");
        for s in &by_lang {
            assert_eq!(s.language, "Go");
        }
    }

    #[test]
    fn test_find_by_name_exact_match() {
        let starters = load_bundled_starters();
        let found = find_by_name(&starters, "Extend Override :: Lootbox Roll :: Go");
        assert!(found.is_some(), "should find by exact name");
        assert_eq!(found.unwrap().language, "Go");
    }

    #[test]
    fn test_find_by_name_returns_none_for_unknown() {
        let starters = load_bundled_starters();
        assert!(find_by_name(&starters, "nonexistent-template").is_none());
    }

    /// Every starter name must be unique. A duplicate would make `find_by_name`
    /// silently return only the first match, leaving the second unreachable.
    #[test]
    fn test_all_starter_names_are_unique() {
        let starters = load_bundled_starters();
        let mut seen = std::collections::HashSet::new();
        for s in &starters {
            assert!(
                seen.insert(&s.name),
                "duplicate starter name in starters.json: '{}'",
                s.name
            );
        }
    }

    #[test]
    fn test_source_path_present_for_ui_templates() {
        let starters = load_bundled_starters();
        let ui = filter_by_scenario(&starters, "Extend App UI");
        assert!(!ui.is_empty(), "should find App UI starters");
        for s in &ui {
            assert!(
                s.source_path.is_some(),
                "App UI starters should have a source_path: {}",
                s.name
            );
        }
    }
}
