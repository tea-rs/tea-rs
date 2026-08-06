#![forbid(unsafe_code)]

#[cfg(feature = "fixture-server")]
mod support;

#[path = "catalog.rs"]
mod catalog;
#[path = "config.rs"]
mod config;
#[path = "conformance.rs"]
mod conformance;
#[path = "content.rs"]
mod content;
#[path = "contracts.rs"]
mod contracts;
#[path = "descriptor.rs"]
mod descriptor;
#[path = "executor.rs"]
mod executor;
#[path = "fault_injection.rs"]
mod fault_injection;
#[path = "health.rs"]
mod health;
#[path = "lifecycle.rs"]
mod lifecycle;
#[path = "process_cleanup.rs"]
mod process_cleanup;
#[path = "reconnect.rs"]
mod reconnect;
#[path = "schema.rs"]
mod schema;
#[path = "secrets.rs"]
mod secrets;
#[path = "security.rs"]
mod security;
#[path = "stdio.rs"]
mod stdio;
