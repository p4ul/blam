# BLAM!

**Type fast. Claim first. Keep the crown.**

BLAM! is a LAN-first, local-first, multiplayer word brawler. Players share the same rack of letters and race to type words before anyone else claims them. First to claim wins the points.

## Features

- **LAN multiplayer** - Auto-discovers players on your local network via mDNS
- **Terminal UI** - Clean, fast interface using Ratatui
- **Local-first** - Works without internet, data stays on your machine
- **Persistent stats** - Lifetime rankings and Elo ratings across sessions
- **Solo practice** - Hone your skills when nobody's around

## Building

Requires Rust 1.88 or later (1.93+ recommended). Use [rustup](https://rustup.rs/) to manage your Rust installation.

```bash
# Using make (recommended - ensures correct toolchain)
make release
./target/release/blam

# Or using the toolchain wrapper directly
./scripts/cargo.sh build --release
./target/release/blam

# Run tests on the pinned toolchain
make test
```

If plain `cargo test` reports errors about `edition2024` with Cargo 1.75, your
system Cargo is taking precedence in `PATH`. Use `make test` or
`./scripts/cargo.sh test` instead.

## How to Play

### Starting a Game

1. Run `blam` on each machine
2. Enter your handle (player name)
3. One player selects **Start Lobby** to host
4. Other players see the lobby via auto-discovery and join
5. Host presses Enter to start the countdown

### Gameplay

- A rack of 12-20 letters appears
- Type words using only those letters
- Press Enter to submit each word
- First player to claim a word scores 1 point per letter
- Round ends after 60 seconds (configurable)

### Word Rules

- Minimum 3 letters
- Must be in the dictionary
- Can only use available letters (respects duplicates)
- Each word can only be claimed once per round

### Controls

| Key | Action |
|-----|--------|
| Arrow keys | Navigate menus |
| Enter | Select / Submit word |
| Tab | Switch fields |
| Esc | Back / Exit (after round ends) |

## Data Storage

BLAM! stores your data in the standard app data location:

- **Linux**: `~/.local/share/blam/`
- **macOS**: `~/Library/Application Support/blam/`

Your history syncs automatically when you reconnect with previous opponents.

## Requirements

- Rust 1.88+ (for building)
- Terminal with 256-color support
- Network access for LAN play (mDNS on port 5353, game on port 55333)

## License

See the `plan` file for the full product specification.
