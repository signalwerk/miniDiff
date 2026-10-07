use std::collections::BTreeMap;

/// Counts words in a text, ignoring punctuation.
fn count_words(text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for word in text.split_whitespace() {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        if word.is_empty() {
            continue;
        }
        *counts.entry(word.to_lowercase()).or_insert(0) += 1;
    }
    counts
}

fn main() {
    let text = "The quick brown fox jumps over the lazy dog. The end!";
    for (word, n) in count_words(text) {
        println!("{word:>8}: {n}");
    }
}
