//! BLAM! - LAN-first, local-first, multiplayer word brawler
//!
//! Type fast. Claim first. Keep the crown.

mod app;
mod game;
mod network;
mod storage;
mod tui;

use app::{App, DEFAULT_ROUND_DURATION};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::io;
use std::time::{Duration, Instant};
use tui::Tui;

fn main() -> io::Result<()> {
    // Initialize terminal
    let mut terminal = Tui::new()?;
    terminal.enter()?;

    // Initialize app with demo letters
    let mut app = App::new();
    app.start_round(
        vec!['B', 'L', 'A', 'M', 'T', 'Y', 'P', 'E', 'R', 'S', 'O', 'N'],
        DEFAULT_ROUND_DURATION,
    );

    // Main event loop
    let tick_rate = Duration::from_secs(1);
    let mut last_tick = Instant::now();

    loop {
        // Render
        terminal.draw(|frame| tui::render(frame, &app))?;

        // Calculate timeout for next tick
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        // Poll for events with timeout
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Only handle key press events (not release)
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Esc => {
                            app.quit();
                        }
                        KeyCode::Enter => {
                            app.on_submit();
                        }
                        KeyCode::Backspace => {
                            app.on_backspace();
                        }
                        KeyCode::Char(c) => {
                            // Only accept alphabetic characters
                            if c.is_ascii_alphabetic() {
                                app.on_char(c.to_ascii_uppercase());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Handle timer tick
        if last_tick.elapsed() >= tick_rate {
            app.tick();
            last_tick = Instant::now();
        }

        // Check for quit
        if app.should_quit {
            break;
        }
    }

    // Terminal cleanup happens automatically via Tui::drop
    Ok(())
}
