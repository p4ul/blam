//! Application state and core logic

pub mod screen;
pub mod state;

pub use screen::{AppCoordinator, MenuOption, Screen};
pub use state::{App, DEFAULT_ROUND_DURATION};
