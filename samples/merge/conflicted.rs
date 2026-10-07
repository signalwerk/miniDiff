fn greet(name: &str) -> String {
<<<<<<< HEAD
    format!("Hello, {name}!")
||||||| base
    format!("Hello, {name}")
=======
    format!("Good day, {name}")
>>>>>>> feature/polite
}

fn main() {
    println!("{}", greet("world"));
}
