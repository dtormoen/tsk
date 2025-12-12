//! Container engine abstraction module
//!
//! This module provides engine-agnostic container operations,
//! supporting both Docker and Podman.

pub mod engine;

pub use engine::{ContainerEngine, EngineConfig, default_socket_path, detect_engine, is_engine_available};
