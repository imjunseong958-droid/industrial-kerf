//! Collaboration on Kerf projects through Git and GitHub. The project is
//! text, so people share it the way code is shared: commits, pulls, pushes,
//! branches, and pull requests. `project.kerf` merges structurally, by the
//! ids of what it holds, through a git merge driver.

mod driver;
pub mod git;
mod merge;
pub mod scaffold;

pub use driver::run_merge_driver;
pub use merge::{merge_documents, merge_projects, summarize, Conflict, Merged, MergedDocument};
