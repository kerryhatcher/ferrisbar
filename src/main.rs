mod payload;
mod layout;

use std::io::Read;

fn main() {
    let mut discard = String::new();
    let _ = std::io::stdin().read_to_string(&mut discard);
    println!("Hello World");
}
