//! BLAM! - LAN-first, local-first, multiplayer word brawler
//!
//! Type fast. Claim first. Keep the crown.

mod app;
mod game;
mod network;
mod storage;
mod tui;

use app::App;

fn main() {
    let _app = App::new();
    println!("BLAM! - Type fast. Claim first. Keep the crown.");
}
