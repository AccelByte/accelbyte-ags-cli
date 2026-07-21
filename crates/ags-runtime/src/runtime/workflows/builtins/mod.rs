//! Built-in workflows registered with the process-wide `WorkflowRegistry`
//! the first time `registry()` is accessed.

pub mod competitive_multiplayer;
pub mod in_game_store;
pub mod player_overview;
pub mod season_pass;

use super::WorkflowRegistry;

/// Register every built-in workflow into `registry`. Called once, inside
/// the `registry()` `OnceLock` init closure.
pub fn register_builtins(registry: &mut WorkflowRegistry) {
    registry.register(Box::new(
        competitive_multiplayer::CompetitiveMultiplayer::new(),
    ));
    registry.register(Box::new(player_overview::PlayerOverview::new()));
    registry.register(Box::new(in_game_store::InGameStore::new()));
    registry.register(Box::new(season_pass::SeasonPass::new()));
}
