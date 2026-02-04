//! Game logic: rounds, scoring, letter racks, word validation

pub mod dictionary;
pub mod validation;

use rand::distr::weighted::WeightedIndex;
use rand::prelude::*;

/// English letter frequencies (percentages * 100 for integer weights).
/// Based on standard English text frequency analysis.
const LETTER_WEIGHTS: [(char, u32); 26] = [
    ('A', 820),
    ('B', 150),
    ('C', 280),
    ('D', 430),
    ('E', 1270),
    ('F', 220),
    ('G', 200),
    ('H', 610),
    ('I', 700),
    ('J', 15),
    ('K', 80),
    ('L', 400),
    ('M', 240),
    ('N', 670),
    ('O', 750),
    ('P', 190),
    ('Q', 10),
    ('R', 600),
    ('S', 630),
    ('T', 910),
    ('U', 280),
    ('V', 100),
    ('W', 240),
    ('X', 15),
    ('Y', 200),
    ('Z', 7),
];

const VOWELS: [char; 5] = ['A', 'E', 'I', 'O', 'U'];
const MIN_VOWELS: usize = 2;
const MIN_RACK_SIZE: usize = 12;
const MAX_RACK_SIZE: usize = 20;

/// A rack of letters for a game round.
#[derive(Debug, Clone)]
pub struct LetterRack {
    letters: Vec<char>,
}

impl LetterRack {
    /// Generate a new random letter rack with 12-20 letters.
    /// Letters are weighted to English frequency.
    /// Guarantees at least 2 vowels by rerolling if needed.
    pub fn generate() -> Self {
        Self::generate_with_rng(&mut rand::rng())
    }

    /// Generate a letter rack using a specific RNG (for testing/seeding).
    pub fn generate_with_rng<R: Rng>(rng: &mut R) -> Self {
        loop {
            let rack = Self::generate_once(rng);
            if rack.vowel_count() >= MIN_VOWELS {
                return rack;
            }
            // Reroll if fewer than 2 vowels
        }
    }

    fn generate_once<R: Rng>(rng: &mut R) -> Self {
        let size = rng.random_range(MIN_RACK_SIZE..=MAX_RACK_SIZE);

        let letters: Vec<char> = LETTER_WEIGHTS.iter().map(|(c, _)| *c).collect();
        let weights: Vec<u32> = LETTER_WEIGHTS.iter().map(|(_, w)| *w).collect();
        let dist = WeightedIndex::new(&weights).expect("valid weights");

        let rack_letters: Vec<char> = (0..size).map(|_| letters[dist.sample(rng)]).collect();

        Self {
            letters: rack_letters,
        }
    }

    /// Count the number of vowels in the rack.
    pub fn vowel_count(&self) -> usize {
        self.letters.iter().filter(|c| VOWELS.contains(c)).count()
    }

    /// Get the letters in the rack.
    pub fn letters(&self) -> &[char] {
        &self.letters
    }

    /// Get the rack size.
    pub fn len(&self) -> usize {
        self.letters.len()
    }

    /// Check if the rack is empty.
    pub fn is_empty(&self) -> bool {
        self.letters.is_empty()
    }

    /// Display the rack as a string.
    pub fn as_string(&self) -> String {
        self.letters.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rack_size_in_range() {
        for _ in 0..100 {
            let rack = LetterRack::generate();
            assert!(rack.len() >= MIN_RACK_SIZE);
            assert!(rack.len() <= MAX_RACK_SIZE);
        }
    }

    #[test]
    fn test_rack_has_minimum_vowels() {
        for _ in 0..100 {
            let rack = LetterRack::generate();
            assert!(
                rack.vowel_count() >= MIN_VOWELS,
                "Rack {} has only {} vowels",
                rack.as_string(),
                rack.vowel_count()
            );
        }
    }

    #[test]
    fn test_rack_contains_only_uppercase_letters() {
        for _ in 0..100 {
            let rack = LetterRack::generate();
            for c in rack.letters() {
                assert!(c.is_ascii_uppercase(), "Found non-uppercase char: {}", c);
            }
        }
    }

    #[test]
    fn test_seeded_generation_is_deterministic() {
        use rand::SeedableRng;

        let mut rng1 = rand::rngs::StdRng::seed_from_u64(42);
        let mut rng2 = rand::rngs::StdRng::seed_from_u64(42);

        let rack1 = LetterRack::generate_with_rng(&mut rng1);
        let rack2 = LetterRack::generate_with_rng(&mut rng2);

        assert_eq!(rack1.as_string(), rack2.as_string());
    }
}
