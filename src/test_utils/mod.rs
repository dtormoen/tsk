//! Test utilities for TSK integration tests.
//!
//! This module provides mock implementations and test helpers for:
//! - Docker client mocks (NoOpDockerClient, FixedResponseDockerClient, TrackedDockerClient)
//! - Git repository test utilities (TestGitRepository, create_files_with_gitignore)
//! - TSK client mocks (NoOpTskClient)

pub mod docker_clients;
pub mod git_test_helpers;
pub mod git_test_utils;
pub mod tsk_clients;

pub use docker_clients::{FixedResponseDockerClient, NoOpDockerClient, TrackedDockerClient};
pub use git_test_helpers::create_files_with_gitignore;
pub use git_test_utils::TestGitRepository;
pub use tsk_clients::NoOpTskClient;
