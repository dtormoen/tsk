use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct Message {
    text: String,
    status: String,
}

fn main() {
    let msg = Message {
        text: "Hello from Rust integration test!".to_string(),
        status: "success".to_string(),
    };
    let json = serde_json::to_string_pretty(&msg).unwrap();
    println!("{}", json);
    println!("SUCCESS: Rust stack is working");
}
