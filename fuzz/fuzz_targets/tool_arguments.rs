#![no_main]

use std::str::FromStr;

use libfuzzer_sys::fuzz_target;
use tea_protocol::{ProtocolMetadata, ToolCallId};
use tea_tools::{ToolInvocation, ToolName};

const MAX_FUZZ_INPUT_BYTES: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }
    let Ok(arguments) = serde_json::from_slice(data) else {
        return;
    };

    let _ = ToolInvocation::new(
        ToolCallId::from_str("0195a0b1-7100-7000-8000-0aa7aa000001").unwrap(),
        ToolName::from_str("bounded_input").unwrap(),
        arguments,
        ProtocolMetadata::default(),
    );
});
