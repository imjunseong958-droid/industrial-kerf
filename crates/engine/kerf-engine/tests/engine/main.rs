//! kerf-engine's integration tests, one test binary, each former test file a module. kept_sound and sections stay binaries of their own: they count renders through the process-wide section counters (`section_stats`), which tests running beside them would move.

mod delivery;
mod eye;
mod follows;
mod footage_color;
mod frame_cache;
mod guide;
mod media_index;
mod models;
mod placed;
mod program_render;
mod recipe_render;
mod shot_fixtures;
mod shows;
mod sound_effects;
mod sound_place;
mod speed_ramp;
mod stands;
