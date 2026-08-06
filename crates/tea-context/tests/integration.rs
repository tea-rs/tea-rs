#![forbid(unsafe_code)]

#[path = "budget.rs"]
mod budget;
#[path = "builtin_providers.rs"]
mod builtin_providers;
#[path = "common/mod.rs"]
mod common;
#[path = "conflicts.rs"]
mod conflicts;
#[path = "contracts.rs"]
mod contracts;
#[path = "determinism.rs"]
mod determinism;
#[path = "inspection.rs"]
mod inspection;
#[path = "modules.rs"]
mod modules;
#[path = "providers.rs"]
mod providers;
#[path = "skills.rs"]
mod skills;
#[path = "tool_hints.rs"]
mod tool_hints;
#[path = "values.rs"]
mod values;
