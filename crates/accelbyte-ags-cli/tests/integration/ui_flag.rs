//! --ui flag accepts plain, inline, fullscreen, auto. The legacy `tui` alias
//! has been removed and is now rejected.

use ags::invocation::flags::UiFlag;
use clap::ValueEnum;

#[test]
fn test_ui_flag_accepts_all_four_canonical_values() {
    for value in ["auto", "plain", "inline", "fullscreen"] {
        UiFlag::from_str(value, true).unwrap_or_else(|e| panic!("expected {value} to parse: {e}"));
    }
}

#[test]
fn test_ui_flag_tui_alias_is_rejected() {
    assert!(
        UiFlag::from_str("tui", true).is_err(),
        "the removed tui alias must no longer parse"
    );
}
