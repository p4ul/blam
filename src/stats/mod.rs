#![allow(dead_code)]
//! Statistics and Elo rating system
//!
//! This module provides:
//! - Pairwise Elo calculations for 2-12 player multiplayer matches
//! - Deterministic replay from match history
//! - Lifetime stats tracking (rounds played, scores, words claimed)
//!
//! Elo is calculated using pairwise comparisons (Section 10 of PRD):
//! - At match end, each player pair gets a win/loss/draw result
//! - Expected score: E_A = 1 / (1 + 10^((R_B - R_A)/400))
//! - Rating update: ΔR_A = (K/(N-1)) * Σ(Result - Expected)

use std::collections::HashMap;

/// Default K factor for Elo calculations
pub const DEFAULT_K: f64 = 32.0;

/// Default starting Elo rating
pub const DEFAULT_ELO: f64 = 1200.0;

/// Match result stored in the event log
#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    /// Unique match ID (timestamp-based for deterministic ordering)
    pub match_id: i64,
    /// Final scores for each player: (player_handle, score)
    pub scores: Vec<(String, u32)>,
    /// Host actor ID that ran this match
    pub host_actor_id: String,
    /// Whether the match completed successfully
    pub completed: bool,
}

impl MatchResult {
    /// Create a new match result
    pub fn new(match_id: i64, scores: Vec<(String, u32)>, host_actor_id: String) -> Self {
        MatchResult {
            match_id,
            scores,
            host_actor_id,
            completed: true,
        }
    }

    /// Parse match result from JSON payload
    pub fn from_json(json: &str) -> Option<Self> {
        // Simple JSON parsing without serde
        let match_id = extract_i64(json, "match_id")?;
        let host_actor_id = extract_string(json, "host_actor_id")?;
        let completed = extract_bool(json, "completed").unwrap_or(true);
        let scores = extract_scores(json)?;

        Some(MatchResult {
            match_id,
            scores,
            host_actor_id,
            completed,
        })
    }

    /// Serialize to JSON payload
    pub fn to_json(&self) -> String {
        let scores_json: String = self
            .scores
            .iter()
            .map(|(name, score)| format!(r#"["{}",{}]"#, escape_json(name), score))
            .collect::<Vec<_>>()
            .join(",");

        format!(
            r#"{{"match_id":{},"scores":[{}],"host_actor_id":"{}","completed":{}}}"#,
            self.match_id,
            scores_json,
            escape_json(&self.host_actor_id),
            self.completed
        )
    }

    /// Get the number of players in this match
    pub fn player_count(&self) -> usize {
        self.scores.len()
    }

    /// Check if this is a multiplayer match (2+ players)
    pub fn is_multiplayer(&self) -> bool {
        self.scores.len() >= 2
    }
}

/// Player lifetime statistics
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerStats {
    /// Player handle
    pub handle: String,
    /// Current Elo rating
    pub elo: f64,
    /// Total rounds played
    pub rounds_played: u32,
    /// Total points scored across all rounds
    pub total_points: u32,
    /// Best single-round score
    pub best_score: u32,
    /// Longest word claimed (by character count)
    pub longest_word: String,
    /// Total words claimed across all rounds
    pub words_claimed: u32,
    /// Number of wins (first place finishes)
    pub wins: u32,
}

impl PlayerStats {
    /// Create stats for a new player
    pub fn new(handle: String) -> Self {
        PlayerStats {
            handle,
            elo: DEFAULT_ELO,
            rounds_played: 0,
            total_points: 0,
            best_score: 0,
            longest_word: String::new(),
            words_claimed: 0,
            wins: 0,
        }
    }

    /// Average score per round
    pub fn average_score(&self) -> f64 {
        if self.rounds_played == 0 {
            0.0
        } else {
            self.total_points as f64 / self.rounds_played as f64
        }
    }
}

/// Elo calculator with deterministic replay
#[derive(Debug, Clone)]
pub struct EloCalculator {
    /// K factor for rating updates
    k_factor: f64,
    /// Current ratings for all known players
    ratings: HashMap<String, f64>,
}

impl EloCalculator {
    /// Create a new Elo calculator
    pub fn new() -> Self {
        EloCalculator {
            k_factor: DEFAULT_K,
            ratings: HashMap::new(),
        }
    }

    /// Create with custom K factor
    pub fn with_k_factor(k_factor: f64) -> Self {
        EloCalculator {
            k_factor,
            ratings: HashMap::new(),
        }
    }

    /// Get a player's current rating (or default if new)
    pub fn rating(&self, player: &str) -> f64 {
        self.ratings.get(player).copied().unwrap_or(DEFAULT_ELO)
    }

    /// Get all ratings
    pub fn all_ratings(&self) -> &HashMap<String, f64> {
        &self.ratings
    }

    /// Get ratings sorted by Elo (highest first)
    pub fn leaderboard(&self) -> Vec<(String, f64)> {
        let mut ratings: Vec<_> = self.ratings.iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        ratings.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ratings
    }

    /// Calculate expected score for player A vs player B
    fn expected_score(rating_a: f64, rating_b: f64) -> f64 {
        1.0 / (1.0 + 10.0_f64.powf((rating_b - rating_a) / 400.0))
    }

    /// Process a match and update ratings
    ///
    /// Uses pairwise comparisons for multiplayer (Section 10.2 of PRD):
    /// - For every pair (A,B): if S_A > S_B, A gets 1.0; tie = 0.5; else 0.0
    /// - ΔR_A = (K/(N-1)) * Σ(Result - Expected)
    pub fn process_match(&mut self, result: &MatchResult) {
        if !result.completed || !result.is_multiplayer() {
            return;
        }

        let n = result.player_count();
        let k_adjusted = self.k_factor / (n - 1) as f64;

        // Collect current ratings for all players
        let player_ratings: Vec<(String, u32, f64)> = result
            .scores
            .iter()
            .map(|(name, score)| {
                let rating = self.rating(name);
                (name.clone(), *score, rating)
            })
            .collect();

        // Calculate rating changes for each player using pairwise comparisons
        let mut rating_changes: HashMap<String, f64> = HashMap::new();

        for (i, (player_a, score_a, rating_a)) in player_ratings.iter().enumerate() {
            let mut total_change = 0.0;

            for (j, (_, score_b, rating_b)) in player_ratings.iter().enumerate() {
                if i == j {
                    continue;
                }

                // Determine actual result
                let actual = match score_a.cmp(score_b) {
                    std::cmp::Ordering::Greater => 1.0,
                    std::cmp::Ordering::Equal => 0.5,
                    std::cmp::Ordering::Less => 0.0,
                };

                // Calculate expected result
                let expected = Self::expected_score(*rating_a, *rating_b);

                // Accumulate change
                total_change += k_adjusted * (actual - expected);
            }

            rating_changes.insert(player_a.clone(), total_change);
        }

        // Apply rating changes
        for (player, change) in rating_changes {
            let current = self.rating(&player);
            self.ratings.insert(player, current + change);
        }
    }

    /// Replay a list of matches in order to compute final ratings
    ///
    /// Matches are sorted by match_id to ensure deterministic results
    /// after CRDT merge (Section 10.3 of PRD)
    pub fn replay_matches(&mut self, matches: &mut [MatchResult]) {
        // Sort by match_id for deterministic ordering
        matches.sort_by_key(|m| m.match_id);

        // Reset ratings
        self.ratings.clear();

        // Process each match in order
        for result in matches {
            self.process_match(result);
        }
    }
}

impl Default for EloCalculator {
    fn default() -> Self {
        Self::new()
    }
}

/// Stats tracker that maintains lifetime statistics for all players
#[derive(Debug, Default)]
pub struct StatsTracker {
    /// Statistics for each player by handle
    stats: HashMap<String, PlayerStats>,
    /// Elo calculator
    elo: EloCalculator,
}

impl StatsTracker {
    /// Create a new stats tracker
    pub fn new() -> Self {
        StatsTracker::default()
    }

    /// Get stats for a player (creates default if not exists)
    pub fn get_or_create(&mut self, handle: &str) -> &mut PlayerStats {
        if !self.stats.contains_key(handle) {
            self.stats.insert(handle.to_string(), PlayerStats::new(handle.to_string()));
        }
        self.stats.get_mut(handle).unwrap()
    }

    /// Get stats for a player (read-only)
    pub fn get(&self, handle: &str) -> Option<&PlayerStats> {
        self.stats.get(handle)
    }

    /// Get all player stats
    pub fn all_stats(&self) -> &HashMap<String, PlayerStats> {
        &self.stats
    }

    /// Process a completed match, updating stats and Elo
    pub fn process_match(&mut self, result: &MatchResult) {
        if !result.completed {
            return;
        }

        // Find winner(s)
        let max_score = result.scores.iter().map(|(_, s)| *s).max().unwrap_or(0);

        // Update stats for each player
        for (handle, score) in &result.scores {
            let stats = self.get_or_create(handle);
            stats.rounds_played += 1;
            stats.total_points += score;
            if *score > stats.best_score {
                stats.best_score = *score;
            }
            if result.is_multiplayer() && *score == max_score {
                stats.wins += 1;
            }
        }

        // Update Elo for multiplayer matches
        if result.is_multiplayer() {
            self.elo.process_match(result);
            // Sync Elo ratings to stats
            for (handle, _) in &result.scores {
                if let Some(stats) = self.stats.get_mut(handle) {
                    stats.elo = self.elo.rating(handle);
                }
            }
        }
    }

    /// Record a word claim (for longest word tracking)
    pub fn record_word_claim(&mut self, handle: &str, word: &str) {
        let stats = self.get_or_create(handle);
        stats.words_claimed += 1;
        if word.len() > stats.longest_word.len() {
            stats.longest_word = word.to_string();
        }
    }

    /// Get Elo leaderboard (sorted by rating)
    pub fn elo_leaderboard(&self) -> Vec<(String, f64)> {
        self.elo.leaderboard()
    }

    /// Get points leaderboard (sorted by total points)
    pub fn points_leaderboard(&self) -> Vec<(String, u32)> {
        let mut leaderboard: Vec<_> = self.stats.iter()
            .map(|(handle, stats)| (handle.clone(), stats.total_points))
            .collect();
        leaderboard.sort_by(|a, b| b.1.cmp(&a.1));
        leaderboard
    }

    /// Rebuild stats from a list of match results
    ///
    /// Used after CRDT sync to recompute stats deterministically
    pub fn rebuild_from_matches(&mut self, matches: &mut [MatchResult]) {
        // Clear existing stats
        self.stats.clear();
        self.elo = EloCalculator::new();

        // Sort and process matches
        matches.sort_by_key(|m| m.match_id);
        for result in matches {
            self.process_match(result);
        }
    }
}

// Helper functions for simple JSON parsing

fn extract_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!(r#""{}":""#, key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..];
    let end = find_unescaped_quote(rest)?;
    Some(unescape_json(&rest[..end]))
}

fn extract_i64(json: &str, key: &str) -> Option<i64> {
    let pattern = format!(r#""{}":"#, key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_bool(json: &str, key: &str) -> Option<bool> {
    let pattern = format!(r#""{}":"#, key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_scores(json: &str) -> Option<Vec<(String, u32)>> {
    let pattern = r#""scores":["#;
    let start = json.find(pattern)? + pattern.len();
    let rest = &json[start..];

    // Find matching closing bracket
    let mut depth = 1;
    let mut end = 0;
    for (i, c) in rest.chars().enumerate() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }

    let array = &rest[..end];
    let mut scores = Vec::new();
    let mut current = array;

    while let Some(start) = current.find('[') {
        let inner = &current[start + 1..];
        if let Some(end) = inner.find(']') {
            let item = &inner[..end];
            // Parse ["name", score]
            if let Some(comma) = item.find(',') {
                let name = item[..comma].trim().trim_matches('"');
                let score_str = item[comma + 1..].trim();
                if let Ok(score) = score_str.parse() {
                    scores.push((unescape_json(name), score));
                }
            }
            current = &inner[end + 1..];
        } else {
            break;
        }
    }

    Some(scores)
}

fn find_unescaped_quote(s: &str) -> Option<usize> {
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            return Some(i);
        } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    None
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn unescape_json(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expected_score() {
        // Equal ratings should give 0.5 expected
        let expected = EloCalculator::expected_score(1200.0, 1200.0);
        assert!((expected - 0.5).abs() < 0.001);

        // Higher rated player should have >0.5 expected
        let expected = EloCalculator::expected_score(1400.0, 1200.0);
        assert!(expected > 0.5);

        // Lower rated player should have <0.5 expected
        let expected = EloCalculator::expected_score(1000.0, 1200.0);
        assert!(expected < 0.5);
    }

    #[test]
    fn test_two_player_match() {
        let mut calc = EloCalculator::new();

        // Player A beats Player B
        let result = MatchResult::new(
            1,
            vec![("Alice".to_string(), 50), ("Bob".to_string(), 30)],
            "host1".to_string(),
        );
        calc.process_match(&result);

        // Winner's rating should increase
        assert!(calc.rating("Alice") > DEFAULT_ELO);
        // Loser's rating should decrease
        assert!(calc.rating("Bob") < DEFAULT_ELO);

        // Rating changes should be equal and opposite (for 2 players)
        let alice_change = calc.rating("Alice") - DEFAULT_ELO;
        let bob_change = calc.rating("Bob") - DEFAULT_ELO;
        assert!((alice_change + bob_change).abs() < 0.001);
    }

    #[test]
    fn test_three_player_match() {
        let mut calc = EloCalculator::new();

        // Alice > Bob > Charlie
        let result = MatchResult::new(
            1,
            vec![
                ("Alice".to_string(), 50),
                ("Bob".to_string(), 30),
                ("Charlie".to_string(), 10),
            ],
            "host1".to_string(),
        );
        calc.process_match(&result);

        // Alice should gain the most
        let alice_rating = calc.rating("Alice");
        let bob_rating = calc.rating("Bob");
        let charlie_rating = calc.rating("Charlie");

        assert!(alice_rating > bob_rating);
        assert!(bob_rating > charlie_rating);
    }

    #[test]
    fn test_tie_handling() {
        let mut calc = EloCalculator::new();

        // Players tie
        let result = MatchResult::new(
            1,
            vec![("Alice".to_string(), 30), ("Bob".to_string(), 30)],
            "host1".to_string(),
        );
        calc.process_match(&result);

        // Equal ratings for equal players tying should stay equal
        let alice = calc.rating("Alice");
        let bob = calc.rating("Bob");
        assert!((alice - bob).abs() < 0.001);
    }

    #[test]
    fn test_deterministic_replay() {
        let mut matches = vec![
            MatchResult::new(3, vec![("A".to_string(), 50), ("B".to_string(), 30)], "h".to_string()),
            MatchResult::new(1, vec![("A".to_string(), 20), ("B".to_string(), 40)], "h".to_string()),
            MatchResult::new(2, vec![("A".to_string(), 30), ("B".to_string(), 30)], "h".to_string()),
        ];

        let mut calc1 = EloCalculator::new();
        calc1.replay_matches(&mut matches.clone());

        // Shuffle and replay - should get same result
        let mut shuffled = vec![matches[1].clone(), matches[2].clone(), matches[0].clone()];
        let mut calc2 = EloCalculator::new();
        calc2.replay_matches(&mut shuffled);

        assert!((calc1.rating("A") - calc2.rating("A")).abs() < 0.001);
        assert!((calc1.rating("B") - calc2.rating("B")).abs() < 0.001);
    }

    #[test]
    fn test_solo_match_ignored() {
        let mut calc = EloCalculator::new();

        // Solo match (1 player)
        let result = MatchResult::new(
            1,
            vec![("Alice".to_string(), 50)],
            "host1".to_string(),
        );
        calc.process_match(&result);

        // Rating should stay at default
        assert!((calc.rating("Alice") - DEFAULT_ELO).abs() < 0.001);
    }

    #[test]
    fn test_match_result_json_roundtrip() {
        let result = MatchResult::new(
            12345,
            vec![
                ("Alice".to_string(), 50),
                ("Bob".to_string(), 30),
            ],
            "actor123".to_string(),
        );

        let json = result.to_json();
        let parsed = MatchResult::from_json(&json).unwrap();

        assert_eq!(result.match_id, parsed.match_id);
        assert_eq!(result.scores, parsed.scores);
        assert_eq!(result.host_actor_id, parsed.host_actor_id);
        assert_eq!(result.completed, parsed.completed);
    }

    #[test]
    fn test_player_stats_average() {
        let mut stats = PlayerStats::new("Alice".to_string());
        stats.rounds_played = 5;
        stats.total_points = 100;

        assert!((stats.average_score() - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_stats_tracker_match_processing() {
        let mut tracker = StatsTracker::new();

        let result = MatchResult::new(
            1,
            vec![
                ("Alice".to_string(), 50),
                ("Bob".to_string(), 30),
            ],
            "host1".to_string(),
        );
        tracker.process_match(&result);

        let alice = tracker.get("Alice").unwrap();
        assert_eq!(alice.rounds_played, 1);
        assert_eq!(alice.total_points, 50);
        assert_eq!(alice.best_score, 50);
        assert_eq!(alice.wins, 1);
        assert!(alice.elo > DEFAULT_ELO);

        let bob = tracker.get("Bob").unwrap();
        assert_eq!(bob.rounds_played, 1);
        assert_eq!(bob.total_points, 30);
        assert_eq!(bob.best_score, 30);
        assert_eq!(bob.wins, 0);
        assert!(bob.elo < DEFAULT_ELO);
    }

    #[test]
    fn test_word_claim_tracking() {
        let mut tracker = StatsTracker::new();

        tracker.record_word_claim("Alice", "CAT");
        tracker.record_word_claim("Alice", "ELEPHANT");
        tracker.record_word_claim("Alice", "DOG");

        let stats = tracker.get("Alice").unwrap();
        assert_eq!(stats.words_claimed, 3);
        assert_eq!(stats.longest_word, "ELEPHANT");
    }

    #[test]
    fn test_elo_leaderboard() {
        let mut tracker = StatsTracker::new();

        // Alice wins against Bob
        tracker.process_match(&MatchResult::new(
            1,
            vec![("Alice".to_string(), 50), ("Bob".to_string(), 30)],
            "h".to_string(),
        ));

        let leaderboard = tracker.elo_leaderboard();
        assert_eq!(leaderboard.len(), 2);
        assert_eq!(leaderboard[0].0, "Alice"); // Alice should be first
        assert_eq!(leaderboard[1].0, "Bob");
    }

    #[test]
    fn test_points_leaderboard() {
        let mut tracker = StatsTracker::new();

        // Multiple matches
        tracker.process_match(&MatchResult::new(
            1,
            vec![("Alice".to_string(), 50), ("Bob".to_string(), 60)],
            "h".to_string(),
        ));
        tracker.process_match(&MatchResult::new(
            2,
            vec![("Alice".to_string(), 40), ("Bob".to_string(), 30)],
            "h".to_string(),
        ));

        let leaderboard = tracker.points_leaderboard();
        // Alice: 90, Bob: 90 - tied
        assert_eq!(leaderboard[0].1, 90);
        assert_eq!(leaderboard[1].1, 90);
    }

    #[test]
    fn test_rebuild_from_matches() {
        let mut matches = vec![
            MatchResult::new(1, vec![("A".to_string(), 50), ("B".to_string(), 30)], "h".to_string()),
            MatchResult::new(2, vec![("A".to_string(), 40), ("B".to_string(), 60)], "h".to_string()),
        ];

        let mut tracker = StatsTracker::new();
        tracker.rebuild_from_matches(&mut matches);

        let a_stats = tracker.get("A").unwrap();
        assert_eq!(a_stats.rounds_played, 2);
        assert_eq!(a_stats.total_points, 90);
        assert_eq!(a_stats.best_score, 50);
        assert_eq!(a_stats.wins, 1);

        let b_stats = tracker.get("B").unwrap();
        assert_eq!(b_stats.rounds_played, 2);
        assert_eq!(b_stats.total_points, 90);
        assert_eq!(b_stats.wins, 1);
    }

    #[test]
    fn test_elo_zero_sum_two_players() {
        // Elo changes should sum to zero for equal-rated players
        let mut calc = EloCalculator::new();
        let result = MatchResult::new(
            1,
            vec![("A".to_string(), 50), ("B".to_string(), 30)],
            "h".to_string(),
        );
        calc.process_match(&result);

        let total_change = (calc.rating("A") - DEFAULT_ELO) + (calc.rating("B") - DEFAULT_ELO);
        assert!(total_change.abs() < 0.001, "Two-player Elo should be zero-sum, got {}", total_change);
    }

    #[test]
    fn test_elo_zero_sum_four_players() {
        // Total Elo change across all players should sum to zero
        let mut calc = EloCalculator::new();
        let result = MatchResult::new(
            1,
            vec![
                ("A".to_string(), 50),
                ("B".to_string(), 40),
                ("C".to_string(), 30),
                ("D".to_string(), 20),
            ],
            "h".to_string(),
        );
        calc.process_match(&result);

        let total: f64 = ["A", "B", "C", "D"]
            .iter()
            .map(|p| calc.rating(p) - DEFAULT_ELO)
            .sum();
        assert!(total.abs() < 0.001, "Multi-player Elo should be zero-sum, got {}", total);
    }

    #[test]
    fn test_incomplete_match_ignored() {
        let mut calc = EloCalculator::new();
        let mut result = MatchResult::new(
            1,
            vec![("A".to_string(), 50), ("B".to_string(), 30)],
            "h".to_string(),
        );
        result.completed = false;
        calc.process_match(&result);

        assert!((calc.rating("A") - DEFAULT_ELO).abs() < 0.001);
        assert!((calc.rating("B") - DEFAULT_ELO).abs() < 0.001);
    }

    #[test]
    fn test_elo_upset_bonus() {
        // Lower-rated player beating higher-rated player should gain more
        let mut calc = EloCalculator::new();

        // First: A beats B (both start at 1200)
        calc.process_match(&MatchResult::new(
            1,
            vec![("A".to_string(), 50), ("B".to_string(), 30)],
            "h".to_string(),
        ));
        let a_after_first = calc.rating("A");
        let b_after_first = calc.rating("B");
        assert!(a_after_first > b_after_first);

        // Second: B (now lower-rated) beats A (now higher-rated)
        let b_before = calc.rating("B");
        calc.process_match(&MatchResult::new(
            2,
            vec![("A".to_string(), 20), ("B".to_string(), 50)],
            "h".to_string(),
        ));
        let b_gain = calc.rating("B") - b_before;

        // B should gain more than the initial 16 (because they're the underdog)
        let a_gain_first = a_after_first - DEFAULT_ELO;
        assert!(b_gain > a_gain_first, "Underdog win should gain more: {} vs {}", b_gain, a_gain_first);
    }

    #[test]
    fn test_elo_custom_k_factor() {
        let mut calc_low_k = EloCalculator::with_k_factor(16.0);
        let mut calc_high_k = EloCalculator::with_k_factor(64.0);

        let result = MatchResult::new(
            1,
            vec![("A".to_string(), 50), ("B".to_string(), 30)],
            "h".to_string(),
        );
        calc_low_k.process_match(&result);
        calc_high_k.process_match(&result);

        let low_k_change = (calc_low_k.rating("A") - DEFAULT_ELO).abs();
        let high_k_change = (calc_high_k.rating("A") - DEFAULT_ELO).abs();
        assert!(high_k_change > low_k_change, "Higher K should produce larger changes");
    }

    #[test]
    fn test_match_result_json_special_chars() {
        let result = MatchResult::new(
            1,
            vec![("O'Brien".to_string(), 50), ("Dr. \"Evil\"".to_string(), 30)],
            "host-actor\"123".to_string(),
        );
        let json = result.to_json();
        let parsed = MatchResult::from_json(&json).unwrap();
        assert_eq!(result.match_id, parsed.match_id);
        assert_eq!(result.host_actor_id, parsed.host_actor_id);
    }

    #[test]
    fn test_match_result_player_count() {
        let single = MatchResult::new(1, vec![("A".to_string(), 50)], "h".to_string());
        assert_eq!(single.player_count(), 1);
        assert!(!single.is_multiplayer());

        let multi = MatchResult::new(1, vec![("A".to_string(), 50), ("B".to_string(), 30)], "h".to_string());
        assert_eq!(multi.player_count(), 2);
        assert!(multi.is_multiplayer());
    }

    #[test]
    fn test_player_stats_zero_rounds() {
        let stats = PlayerStats::new("Alice".to_string());
        assert_eq!(stats.average_score(), 0.0);
        assert_eq!(stats.elo, DEFAULT_ELO);
        assert_eq!(stats.rounds_played, 0);
    }

    #[test]
    fn test_stats_tracker_solo_match_no_wins() {
        let mut tracker = StatsTracker::new();
        let result = MatchResult::new(
            1,
            vec![("Alice".to_string(), 50)],
            "h".to_string(),
        );
        tracker.process_match(&result);

        let alice = tracker.get("Alice").unwrap();
        assert_eq!(alice.rounds_played, 1);
        assert_eq!(alice.total_points, 50);
        // Solo match: no wins counted
        assert_eq!(alice.wins, 0);
        // Solo match: Elo unchanged
        assert!((alice.elo - DEFAULT_ELO).abs() < 0.001);
    }

    #[test]
    fn test_stats_tracker_multiple_matches() {
        let mut tracker = StatsTracker::new();

        // Match 1: Alice wins
        tracker.process_match(&MatchResult::new(
            1, vec![("Alice".to_string(), 50), ("Bob".to_string(), 30)], "h".to_string(),
        ));
        // Match 2: Bob wins
        tracker.process_match(&MatchResult::new(
            2, vec![("Alice".to_string(), 20), ("Bob".to_string(), 60)], "h".to_string(),
        ));
        // Match 3: Tie
        tracker.process_match(&MatchResult::new(
            3, vec![("Alice".to_string(), 40), ("Bob".to_string(), 40)], "h".to_string(),
        ));

        let alice = tracker.get("Alice").unwrap();
        assert_eq!(alice.rounds_played, 3);
        assert_eq!(alice.total_points, 110);
        assert_eq!(alice.best_score, 50);
        assert_eq!(alice.wins, 2); // Won match 1, tied match 3 (both get a win for max score)

        let bob = tracker.get("Bob").unwrap();
        assert_eq!(bob.rounds_played, 3);
        assert_eq!(bob.total_points, 130);
        assert_eq!(bob.best_score, 60);
    }

    #[test]
    fn test_leaderboard_default_calculator() {
        let calc = EloCalculator::default();
        assert!(calc.leaderboard().is_empty());
        assert!(calc.all_ratings().is_empty());
    }

    #[test]
    fn test_rebuild_clears_previous() {
        let mut tracker = StatsTracker::new();

        // First set of matches
        tracker.process_match(&MatchResult::new(
            1, vec![("A".to_string(), 50), ("B".to_string(), 30)], "h".to_string(),
        ));
        assert_eq!(tracker.get("A").unwrap().rounds_played, 1);

        // Rebuild from different matches
        let mut new_matches = vec![
            MatchResult::new(10, vec![("X".to_string(), 50), ("Y".to_string(), 30)], "h".to_string()),
        ];
        tracker.rebuild_from_matches(&mut new_matches);

        // Old players should be gone
        assert!(tracker.get("A").is_none());
        assert!(tracker.get("B").is_none());

        // New players should be present
        assert_eq!(tracker.get("X").unwrap().rounds_played, 1);
        assert_eq!(tracker.get("Y").unwrap().rounds_played, 1);
    }
}
