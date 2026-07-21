//! Fullscreen workflow surface.
//!
//! Layout: header + step strip across the top, main area on the left
//! flexing to fill, fixed-width summary panel on the right, single-line
//! nav bar at the bottom. Submodules:
//!
//! - `lifecycle` — terminal acquire/release on the alternate screen
//!   (parallel to `inline::lifecycle`).
//! - `surface` — `FullscreenSurface`: owns the terminal + render model and
//!   exposes the single `render()` shared by the frontend and interaction.
//! - `layout` — region split helper.
//! - `nav` — contextual bottom nav bar.
//! - `step_strip` / `summary` — top and right widgets.
//! - `phases` — per-phase main-area widgets.
//! - `frontend` — `Frontend` trait impl that drives the surface.

pub mod frontend;
pub mod header;
pub mod interaction;
pub mod layout;
pub mod lifecycle;
pub mod nav;
pub mod phases;
pub mod step_strip;
pub mod summary;
pub mod surface;
