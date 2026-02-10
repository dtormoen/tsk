use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};

// Since this is a binary crate, we need to include the modules directly
mod agent {
    use async_trait::async_trait;

    #[async_trait]
    pub trait LogProcessor: Send + Sync {
        fn process_line(&mut self, line: &str) -> Option<String>;
        fn get_final_result(&self) -> Option<&TaskResult>;
    }

    #[derive(Debug, Clone)]
    pub struct TaskResult {
        pub success: bool,
        pub message: String,
        pub cost_usd: Option<f64>,
        pub duration_ms: Option<u64>,
    }

    pub mod codex {
        #[allow(unused_imports)]
        pub use super::super::codex_log_processor::CodexLogProcessor;
    }
}

// Include the log processor module
#[path = "../agent/codex/codex_log_processor.rs"]
mod codex_log_processor;

use agent::LogProcessor;
use codex_log_processor::CodexLogProcessor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} <log_file_path>", args[0]);
        std::process::exit(1);
    }

    let log_file_path = &args[1];
    let file = File::open(log_file_path)?;
    let reader = BufReader::new(file);

    let mut processor = CodexLogProcessor::new(Some("test-task".to_string()));

    for line in reader.lines() {
        let line = line?;
        if let Some(formatted) = processor.process_line(&line) {
            println!("{}", formatted);
        }
    }

    if let Some(result) = processor.get_final_result() {
        println!("\n=== Final Result ===");
        println!("Success: {}", result.success);
        println!("Message: {}", result.message);
        if let Some(cost) = result.cost_usd {
            println!("Cost: ${:.4}", cost);
        }
        if let Some(duration) = result.duration_ms {
            println!("Duration: {}ms", duration);
        }
    }

    Ok(())
}
