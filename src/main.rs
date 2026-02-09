//! BLAM! - LAN-first, local-first, multiplayer word brawler
//!
//! Type fast. Claim first. Keep the crown.

#[allow(dead_code)]
mod app;
#[allow(dead_code)]
mod game;
#[allow(dead_code)]
mod lobby;
#[allow(dead_code)]
mod network;
#[allow(dead_code)]
mod stats;
#[allow(dead_code)]
mod storage;
#[allow(dead_code)]
mod tui;

use app::{AppCoordinator, Screen, DEFAULT_ROUND_DURATION};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use game::LetterRack;
use std::io;
use std::mem;
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
            let mut host_round_start = None;

            match &mut coordinator.screen {
                Screen::Playing { app, .. } => {
                    app.tick();
                }
                Screen::HostLobby { lobby, countdown } => {
                    if countdown.is_some() {
                        if let Some(event) = lobby.tick_countdown() {
                            match event {
                                lobby::LobbyEvent::Countdown {
                                    countdown: count, ..
                                } => {
                                    *countdown = Some(count);
                                }
                                lobby::LobbyEvent::RoundStart { letters, duration } => {
                                    host_round_start = Some((letters, duration));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }

            // Handle host transition outside the match to allow taking ownership
            if let Some((letters, duration)) = host_round_start {
                let old_screen = mem::replace(
                    &mut coordinator.screen,
                    Screen::Error {
                        message: String::new(),
                    },
                );
                if let Screen::HostLobby { lobby, .. } = old_screen {
                    let player_names: Vec<String> =
                        lobby.players().iter().map(|p| p.name.clone()).collect();
                    let host_name = lobby.host_name.clone();

                    let mut app = app::App::new();
                    app.set_player_name(host_name);
                    app.set_scoreboard(player_names);
                    app.start_round(letters, duration);

                    coordinator.screen = Screen::Playing {
                        app,
                        is_host: true,
                        hosted_lobby: Some(lobby),
                        joined_lobby: None,
                    };
                }
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
                coordinator.quit_hosting();
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
        Screen::Playing {
            app,
            hosted_lobby,
            joined_lobby,
            ..
        } => match code {
            KeyCode::Esc => {
                if app.is_round_over() {
                    coordinator.go_to_menu();
                } else if hosted_lobby.is_some() {
                    coordinator.quit_hosting();
                }
            }
            KeyCode::Enter => {
                if let Some(word) = app.get_pending_claim() {
                    if let Some(lobby) = hosted_lobby {
                        // Host: arbitrate locally and broadcast
                        if let Some(events) = lobby.host_claim(&word) {
                            for event in events {
                                match event {
                                    lobby::LobbyEvent::ClaimAccepted {
                                        word,
                                        player_name,
                                        points,
                                    } => {
                                        app.on_claim_accepted(word, player_name, points);
                                    }
                                    lobby::LobbyEvent::ClaimRejected { word, reason } => {
                                        app.on_claim_rejected(
                                            word,
                                            app::AppCoordinator::map_reject_reason_pub(reason),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                        app.clear_input();
                    } else if let Some(lobby) = joined_lobby {
                        // Client: send claim to host
                        let _ = lobby.send_claim(&word);
                        app.clear_input();
                    } else {
                        // Solo: local validation
                        app.on_submit();
                    }
                }
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
        Screen::Rankings { .. } => match code {
            KeyCode::Esc => coordinator.go_to_menu(),
            KeyCode::Up => coordinator.rankings_up(),
            KeyCode::Down => coordinator.rankings_down(),
            _ => {}
        },
        Screen::Settings { .. } => match code {
            KeyCode::Esc => coordinator.go_to_menu(),
            KeyCode::Enter => coordinator.settings_save(),
            KeyCode::Backspace => coordinator.settings_backspace(),
            KeyCode::Char(c) if c.is_ascii_alphanumeric() || c == '_' => {
                coordinator.settings_char(c)
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
