use std::collections::HashMap;

/// Counts words in a text.
fn count_words(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for word in text.split_whitespace() {
        *counts.entry(word.to_lowercase()).or_insert(0) += 1;
    }
    counts
}

fn main() {
    let text = "the quick brown fox jumps over the lazy dog";
    let counts = count_words(text);
    let mut words: Vec<_> = counts.into_iter().collect();
    words.sort();
    for (word, n) in words {
        println!("{word}: {n}");
    }
}
