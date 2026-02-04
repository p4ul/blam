//! BLAM! - LAN-first, local-first, multiplayer word brawler
//!
//! Type fast. Claim first. Keep the crown.

mod app;
mod game;
mod lobby;
mod network;
mod stats;
mod storage;
mod tui;

use app::{AppCoordinator, Screen, DEFAULT_ROUND_DURATION};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use game::LetterRack;
use std::io;
use std::time::{Duration, Instant};
use tui::Tui;

fn main() -> io::Result<()> {
    // Initialize terminal
    let mut terminal = Tui::new()?;
    terminal.enter()?;

    // Initialize app coordinator
    let mut coordinator = AppCoordinator::new();

    // Main event loop
    let tick_rate = Duration::from_millis(100); // Faster for responsive UI
    let mut last_tick = Instant::now();
    let mut last_second = Instant::now();

    loop {
        // Render
        terminal.draw(|frame| tui::render(frame, &coordinator))?;

        // Calculate timeout for next tick
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        // Poll for events with timeout
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Only handle key press events (not release)
                if key.kind == KeyEventKind::Press {
                    handle_key(&mut coordinator, key.code);
                }
            }
        }

        // Handle updates
        if last_tick.elapsed() >= tick_rate {
            coordinator.poll();
            last_tick = Instant::now();
        }

        // Handle second-based timer for game play and countdown
        if last_second.elapsed() >= Duration::from_secs(1) {
            match &mut coordinator.screen {
                Screen::Playing { app, .. } => {
                    app.tick();
                }
                Screen::HostLobby { lobby, countdown } => {
                    if countdown.is_some() {
                        // Tick the countdown
                        if let Some(event) = lobby.tick_countdown() {
                            match event {
                                lobby::LobbyEvent::Countdown {
                                    countdown: count, ..
                                } => {
                                    *countdown = Some(count);
                                }
                                lobby::LobbyEvent::RoundStart { letters, duration } => {
                                    // Countdown finished - transition to playing
                                    let mut app = app::App::new();
                                    app.start_round(letters, duration);
                                    coordinator.screen = Screen::Playing {
                                        app,
                                        is_host: true,
                                        hosted_lobby: None, // TODO: keep lobby alive for arbitration
                                    };
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
            last_second = Instant::now();
        }

        // Check for quit
        if coordinator.should_quit {
            break;
        }
    }

    // Terminal cleanup happens automatically via Tui::drop
    Ok(())
}

fn handle_key(coordinator: &mut AppCoordinator, code: KeyCode) {
    match &mut coordinator.screen {
        Screen::Menu { editing_handle, .. } => {
            if *editing_handle {
                // Handle editing mode
                match code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Tab => coordinator.menu_tab(),
                    KeyCode::Backspace => coordinator.menu_backspace(),
                    KeyCode::Char(c) if c.is_ascii_alphanumeric() || c == '_' => {
                        coordinator.menu_char(c)
                    }
                    _ => {}
                }
            } else {
                // Handle navigation mode
                match code {
                    KeyCode::Esc => coordinator.quit(),
                    KeyCode::Up => coordinator.menu_up(),
                    KeyCode::Down => coordinator.menu_down(),
                    KeyCode::Enter => coordinator.menu_select(),
                    KeyCode::Tab => coordinator.menu_tab(),
                    _ => {}
                }
            }
        }
        Screen::Browser { .. } => match code {
            KeyCode::Esc => coordinator.go_to_menu(),
            KeyCode::Up => coordinator.browser_up(),
            KeyCode::Down => coordinator.browser_down(),
            KeyCode::Enter => coordinator.browser_select(),
            _ => {}
        },
        Screen::HostLobby { lobby, countdown } => match code {
            KeyCode::Esc => {
                // TODO: Clean shutdown of lobby
                coordinator.go_to_menu();
            }
            KeyCode::Enter => {
                // Only start countdown if we're not already counting down
                if lobby.can_start() && countdown.is_none() {
                    // Generate letters and start countdown
                    let letters = LetterRack::generate().letters().to_vec();
                    let count = lobby.start_countdown(letters, DEFAULT_ROUND_DURATION);
                    *countdown = Some(count);
                }
            }
            _ => {}
        },
        Screen::JoinedLobby { .. } => match code {
            KeyCode::Esc => {
                // Leave the lobby
                coordinator.go_to_menu();
            }
            _ => {}
        },
        Screen::Playing { app, .. } => match code {
            KeyCode::Esc => {
                if app.is_round_over() {
                    coordinator.go_to_menu();
                }
            }
            KeyCode::Enter => {
                app.on_submit();
            }
            KeyCode::Backspace => {
                app.on_backspace();
            }
            KeyCode::Char(c) => {
                if c.is_ascii_alphabetic() {
                    app.on_char(c.to_ascii_uppercase());
                }
            }
            _ => {}
        },
        Screen::Error { .. } => match code {
            KeyCode::Esc => coordinator.go_to_menu(),
            KeyCode::Enter => coordinator.go_to_menu(),
            _ => {}
        },
    }
}
