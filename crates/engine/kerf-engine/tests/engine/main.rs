//! kerf-engine's integration tests, one test binary, each former test file a module. frame_cache, kept_sound and sections stay binaries of their own: they set KERF_FRAME_CACHE or KERF_SECTION_CACHE for the whole process, and kept_sound reads the process-wide section counters.

mod delivery;
mod eye;
mod follows;
mod footage_color;
mod guide;
mod media_index;
mod placed;
mod program_render;
mod recipe_render;
mod shot_fixtures;
mod shows;
mod sound_effects;
mod sound_place;
mod speed_ramp;
mod stands;
