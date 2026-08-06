#![no_main]

use libfuzzer_sys::fuzz_target;
use tea_protocol::{CommandEnvelope, EventEnvelope, RecordEnvelope};

const MAX_FUZZ_INPUT_BYTES: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }

    let _ = serde_json::from_slice::<CommandEnvelope>(data);
    let _ = serde_json::from_slice::<EventEnvelope>(data);
    let _ = serde_json::from_slice::<RecordEnvelope>(data);

    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = CommandEnvelope::decode_value(value.clone());
        let _ = RecordEnvelope::decode_value(value);
    }
});
