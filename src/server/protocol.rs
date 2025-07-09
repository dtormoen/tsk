use crate::task::{Task, TaskStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Request messages that can be sent to the TSK server
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// Add a new task to the queue
    AddTask { repo_path: PathBuf, task: Box<Task> },
    /// List all tasks
    ListTasks,
    /// Get the status of a specific task
    GetStatus { task_id: String },
    /// Shutdown the server
    Shutdown,
}

/// Response messages from the TSK server
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Successful operation
    Success { message: String },
    /// Error occurred
    Error { message: String },
    /// List of tasks
    TaskList { tasks: Vec<Task> },
    /// Status of a specific task
    TaskStatus { status: TaskStatus },
}

impl Request {
    /// Parse a request from a JSON string
    ///
    /// This is a convenience method that wraps serde_json::from_str
    /// for consistent API usage and testing.
    #[cfg(test)]
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize the request to JSON
    ///
    /// This is a convenience method that wraps serde_json::to_string
    /// for consistent API usage and testing.
    #[cfg(test)]
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl Response {
    /// Parse a response from a JSON string
    ///
    /// This is a convenience method that wraps serde_json::from_str
    /// for consistent API usage and testing.
    #[cfg(test)]
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize the response to JSON
    ///
    /// This is a convenience method that wraps serde_json::to_string
    /// for consistent API usage and testing.
    #[cfg(test)]
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let request = Request::ListTasks;
        let json = request.to_json().unwrap();
        let parsed: Request = Request::from_json(&json).unwrap();

        match parsed {
            Request::ListTasks => (),
            _ => panic!("Unexpected request type"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let response = Response::Success {
            message: "Test message".to_string(),
        };
        let json = response.to_json().unwrap();
        let parsed: Response = Response::from_json(&json).unwrap();

        match parsed {
            Response::Success { message } => {
                assert_eq!(message, "Test message");
            }
            _ => panic!("Unexpected response type"),
        }
    }
}
