use serde_json::json;
use tea_coding_tools::{
    EditTool, FindTool, GrepTool, LsTool, ReadTool, WorkspaceFileResourceResolver, WriteTool,
};
use tea_protocol::ToolIdempotency;
use tea_tools::{
    CompiledToolSchema, SchedulerClass, ToolEffect, ToolResourceAccess, ToolResourceResolver,
    ToolRetrySafety,
};

#[test]
fn file_tool_specs_compile_and_declare_conservative_semantics() {
    let read = ReadTool::spec().unwrap();
    let write = WriteTool::spec().unwrap();
    let edit = EditTool::spec().unwrap();
    let grep = GrepTool::spec().unwrap();
    let find = FindTool::spec().unwrap();
    let ls = LsTool::spec().unwrap();

    for spec in [&read, &write, &edit, &grep, &find, &ls] {
        CompiledToolSchema::compile(spec.input_schema().clone()).unwrap();
        CompiledToolSchema::compile(spec.output_schema().clone()).unwrap();
        assert_eq!(spec.version().to_string(), "1.0.0");
        assert!(spec.execution().timeout().as_millis() > 0);
        assert!(spec.prompt_hint().is_some());
    }

    assert_eq!(read.name().as_str(), "read");
    assert_eq!(read.effects(), &[ToolEffect::FsRead]);
    assert_eq!(read.scheduler_class(), SchedulerClass::ParallelReadOnly);
    assert_eq!(read.execution().idempotency(), ToolIdempotency::Idempotent);
    assert_eq!(read.execution().retry_safety(), ToolRetrySafety::Automatic);

    for spec in [&grep, &find, &ls] {
        assert_eq!(spec.effects(), &[ToolEffect::FsRead]);
        assert_eq!(spec.scheduler_class(), SchedulerClass::ParallelReadOnly);
        assert_eq!(spec.execution().retry_safety(), ToolRetrySafety::Automatic);
    }

    assert_eq!(write.name().as_str(), "write");
    assert_eq!(write.effects(), &[ToolEffect::FsWrite]);
    assert_eq!(write.scheduler_class(), SchedulerClass::Serial);
    assert_eq!(write.execution().idempotency(), ToolIdempotency::Idempotent);

    assert_eq!(edit.name().as_str(), "edit");
    assert_eq!(edit.effects(), &[ToolEffect::FsRead, ToolEffect::FsWrite]);
    assert_eq!(edit.scheduler_class(), SchedulerClass::Serial);
    assert_eq!(
        edit.execution().retry_safety(),
        ToolRetrySafety::ExplicitOnly
    );
}

#[test]
fn schemas_reject_unknown_or_out_of_bound_arguments_before_execution() {
    let read =
        CompiledToolSchema::compile(ReadTool::spec().unwrap().input_schema().clone()).unwrap();
    assert!(read.validate(&json!({"path":"src/lib.rs"})).is_ok());
    assert!(
        read.validate(&json!({"path":"src/lib.rs","offset":1,"limit":20}))
            .is_ok()
    );
    assert!(
        read.validate(&json!({"path":"src/lib.rs","extra":true}))
            .is_err()
    );
    assert!(
        read.validate(&json!({"path":"src/lib.rs","offset":0}))
            .is_err()
    );

    let edit =
        CompiledToolSchema::compile(EditTool::spec().unwrap().input_schema().clone()).unwrap();
    assert!(
        edit.validate(&json!({"path":"a","oldText":"x","newText":"y"}))
            .is_ok()
    );
    assert!(
        edit.validate(&json!({"path":"a","oldText":"","newText":"y"}))
            .is_err()
    );
    assert!(
        edit.validate(&json!({
            "path":"a","oldText":"x","newText":"y","expectedReplacements":0
        }))
        .is_err()
    );
}

#[test]
fn resource_resolver_normalizes_workspace_relative_file_locators() {
    let resolver = WorkspaceFileResourceResolver::new(ToolResourceAccess::Write);
    let resources = resolver
        .resolve(
            ReadTool::spec().unwrap().name(),
            &json!({"path":"src/./lib.rs"}),
        )
        .unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].scheme(), "file");
    assert_eq!(resources[0].locator(), "/workspace/src/lib.rs");
    assert_eq!(resources[0].access(), ToolResourceAccess::Write);

    let read_write = WorkspaceFileResourceResolver::read_write()
        .resolve(
            EditTool::spec().unwrap().name(),
            &json!({"path":"src/lib.rs"}),
        )
        .unwrap();
    assert_eq!(read_write.len(), 2);
    assert_eq!(read_write[0].access(), ToolResourceAccess::Read);
    assert_eq!(read_write[1].access(), ToolResourceAccess::Write);

    assert!(
        resolver
            .resolve(
                ReadTool::spec().unwrap().name(),
                &json!({"path":"../outside"}),
            )
            .is_err()
    );
}
