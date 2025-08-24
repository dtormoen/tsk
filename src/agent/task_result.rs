/// Result of a task execution from an AI agent
///
/// This struct is part of the public API and all fields may be accessed by consumers.
/// The cost_usd and duration_ms fields provide useful metadata about task execution.
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// Whether the task completed successfully
    pub success: bool,
    /// Message describing the task result
    pub message: String,
    /// Estimated cost in USD for the task execution
    #[allow(dead_code)] // Available for future use
    pub cost_usd: Option<f64>,
    /// Duration of the task execution in milliseconds
    #[allow(dead_code)] // Available for future use
    pub duration_ms: Option<u64>,
}
