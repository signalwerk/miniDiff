fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", greet("world"));
    println!("{}", add(1, 2));
    println!("done");
}
