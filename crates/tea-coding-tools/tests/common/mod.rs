#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt;
use serde_json::Value;
use tea_coding_tools::{
    EditTool, ReadTool, WorkspaceFileResourceResolver, WorkspaceRoot, WriteTool,
};
use tea_control::CancellationScope;
use tea_protocol::{ProtocolMetadata, ToolCallId};
use tea_tools::{
    ToolExecutionEvent, ToolInvocation, ToolName, ToolRegistry, ToolResourceAccess,
    ToolStreamValidator,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    pub fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("tea-coding-file-tools-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn workspace(&self) -> WorkspaceRoot {
        WorkspaceRoot::new(&self.path).unwrap()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn read_registry(workspace: WorkspaceRoot) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            ReadTool::spec().unwrap(),
            Arc::new(WorkspaceFileResourceResolver::new(ToolResourceAccess::Read)),
            Arc::new(ReadTool::new(workspace)),
        )
        .unwrap();
    registry
}

pub fn write_registry(workspace: WorkspaceRoot) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            WriteTool::spec().unwrap(),
            Arc::new(WorkspaceFileResourceResolver::new(
                ToolResourceAccess::Write,
            )),
            Arc::new(WriteTool::new(workspace)),
        )
        .unwrap();
    registry
}

pub fn edit_registry(workspace: WorkspaceRoot) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry
        .register(
            EditTool::spec().unwrap(),
            Arc::new(WorkspaceFileResourceResolver::read_write()),
            Arc::new(EditTool::new(workspace)),
        )
        .unwrap();
    registry
}

pub fn invocation(name: &str, arguments: Value) -> ToolInvocation {
    ToolInvocation::new(
        ToolCallId::from_str("0195a0b1-5e45-75be-8284-0aa7aa000011").unwrap(),
        ToolName::from_str(name).unwrap(),
        arguments,
        ProtocolMetadata::default(),
    )
    .unwrap()
}

pub async fn execute(
    registry: &ToolRegistry,
    name: &str,
    arguments: Value,
) -> Vec<ToolExecutionEvent> {
    let events = registry
        .execute(invocation(name, arguments), CancellationScope::new())
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    let mut validator = ToolStreamValidator::new();
    for event in &events {
        validator.observe(event).unwrap();
    }
    assert_eq!(validator.finish().unwrap(), events.len());
    events
}

pub fn finished(events: &[ToolExecutionEvent]) -> &tea_tools::ToolResult {
    assert_eq!(events.len(), 1, "unexpected events: {events:?}");
    match &events[0] {
        ToolExecutionEvent::Finished(result) => result,
        event => panic!("expected successful result, got {event:?}"),
    }
}

pub fn failed(events: &[ToolExecutionEvent]) -> &tea_tools::ToolExecutionFailure {
    assert_eq!(events.len(), 1, "unexpected events: {events:?}");
    match &events[0] {
        ToolExecutionEvent::Failed(failure) => failure,
        event => panic!("expected failure, got {event:?}"),
    }
}
