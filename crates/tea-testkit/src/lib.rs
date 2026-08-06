#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Deterministic test doubles and conformance utilities for `tea-rs`.
//!
//! # Example
//!
//! ```
//! use tea_testkit::ScriptedModelResponse;
//!
//! let response = ScriptedModelResponse::text(["deterministic ", "answer"]);
//! assert_eq!(response.steps().len(), 4);
//! ```

mod model;
mod tool;

pub use model::{
    CollectedModelStream, ModelConformanceError, ModelConformanceReport, ModelTerminalKind,
    ScriptStep, ScriptedModelProvider, ScriptedModelResponse, ScriptedProviderError,
    collect_model_stream, run_cancelled_model_provider_case, run_model_provider_case,
};
pub use tool::{
    CollectedToolExecution, FakeProcessScript, FakeProcessTool, FakeReadTool, FakeToolStateError,
    FakeWriteTool, ToolConformanceError, ToolConformanceReport, ToolTerminalKind,
    collect_tool_execution,
};
