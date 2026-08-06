use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const SUPPORTED_FIXTURE_SETS: [(&str, &str); 2] = [("v1.0", "1.0"), ("v1.1", "1.1")];

fn fixture_dir(version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{version}"))
}

#[test]
fn manifest_lists_valid_json_fixtures() {
    for (version_dir, protocol_version) in SUPPORTED_FIXTURE_SETS {
        let fixture_dir = fixture_dir(version_dir);
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(fixture_dir.join("manifest.json")).expect("read fixture manifest"),
        )
        .expect("parse fixture manifest");

        assert_eq!(manifest["protocolVersion"], protocol_version);
        let fixtures = manifest["fixtures"]
            .as_array()
            .expect("fixtures must be an array");
        assert!(!fixtures.is_empty(), "fixture manifest must not be empty");

        let mut listed = BTreeSet::new();
        for fixture in fixtures {
            let category = fixture["category"]
                .as_str()
                .expect("fixture category must be a string");
            assert!(
                matches!(category, "command" | "event" | "record" | "error"),
                "unsupported fixture category: {category}"
            );

            let file = fixture["file"]
                .as_str()
                .expect("fixture file must be a string");
            assert!(
                listed.insert(file.to_owned()),
                "duplicate fixture entry: {file}"
            );
            assert!(
                file.starts_with(&format!("{category}-")),
                "fixture category/name mismatch: {file}"
            );
            let contents = fs::read_to_string(fixture_dir.join(file)).expect("read listed fixture");
            let value: Value = serde_json::from_str(&contents).expect("parse listed fixture");
            assert_eq!(value["protocolVersion"], protocol_version, "fixture {file}");
            assert!(value["type"].is_string(), "fixture {file} needs a type");
        }

        let files = fs::read_dir(&fixture_dir)
            .expect("read fixture directory")
            .map(|entry| entry.expect("read fixture entry").file_name())
            .filter_map(|name| name.to_str().map(str::to_owned))
            .filter(|name| {
                Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                    && name != "manifest.json"
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            listed, files,
            "manifest must list every golden fixture exactly once"
        );
    }
}
