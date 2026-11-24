//! Friendly random container names (`brave_turing`).

const ADJECTIVES: &[&str] = &[
    "brave", "calm", "clever", "cosmic", "eager", "fancy", "gentle", "happy", "jolly", "keen",
    "lucid", "mellow", "nimble", "proud", "quiet", "quirky", "rapid", "serene", "sharp", "sleepy",
    "sunny", "swift", "tender", "vivid", "witty", "zen", "bold", "crisp", "dreamy", "frosty",
];

const NOUNS: &[&str] = &[
    "lovelace", "turing", "hopper", "curie", "tesla", "noether", "ritchie", "thompson", "knuth",
    "dijkstra", "hamilton", "babbage", "shannon", "goodall", "franklin", "meitner", "ramanujan",
    "galileo", "kepler", "darwin", "euler", "gauss", "hypatia", "lamarr", "torvalds", "wozniak",
    "liskov", "perlman", "sammet", "johnson",
];

/// Random name, deterministic in `seed` (pass the container id).
pub fn generate(seed: &str) -> String {
    let n = u64::from_str_radix(&seed[..seed.len().min(12)], 16).unwrap_or(7);
    let a = ADJECTIVES[(n % ADJECTIVES.len() as u64) as usize];
    let b = NOUNS[((n / 97) % NOUNS.len() as u64) as usize];
    format!("{a}_{b}")
}

/// Docker's rule: `[a-zA-Z0-9][a-zA-Z0-9_.-]*`.
pub fn valid(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

#[cfg(test)]
mod tests {
    #[test]
    fn names() {
        let n = super::generate("0123456789abcdef");
        assert!(n.contains('_'));
        assert!(super::valid(&n));
        assert!(super::valid("web-1.a_b"));
        assert!(!super::valid("-x"));
        assert!(!super::valid("a b"));
    }
}
