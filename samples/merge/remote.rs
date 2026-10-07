/// Greets someone politely.
fn greet(name: &str) -> String {
    format!("Good day, {name}")
}

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn main() {
    println!("{}", greet("world"));
    println!("{}", add(1, 2));
}
