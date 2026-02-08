//! Test utilities for TSK integration tests.
//!
//! This module provides mock implementations and test helpers for:
//! - Docker client mocks (NoOpDockerClient, FixedResponseDockerClient, TrackedDockerClient)
//! - Git repository test utilities (TestGitRepository, create_files_with_gitignore)

pub mod docker_clients;
pub mod git_test_helpers;
pub mod git_test_utils;

pub use docker_clients::{FixedResponseDockerClient, NoOpDockerClient, TrackedDockerClient};
pub use git_test_helpers::create_files_with_gitignore;
pub use git_test_utils::{ExistingGitRepository, TestGitRepository};
