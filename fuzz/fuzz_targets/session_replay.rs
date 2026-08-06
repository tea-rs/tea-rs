#![no_main]

use libfuzzer_sys::fuzz_target;
use tea_session::{SessionArchive, SessionReducer};

const MAX_FUZZ_INPUT_BYTES: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(archive) = SessionArchive::decode_json(input) else {
        return;
    };

    let _ = SessionReducer::replay(archive.records().iter().cloned());
});
