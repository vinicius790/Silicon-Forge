//! Typed snapshot errors: removing any field of a captured image either still loads
//! (optional in that version) or fails with exactly `MissingField(that key)`.

use mos6502::snapshot::{MachineImage, SnapshotError};
use mos6502::{Cpu, FlatMemory};

#[test]
fn every_removed_field_is_reported_by_name() {
    let json = MachineImage::capture(&Cpu::new(FlatMemory::new())).to_json();
    let mut keys = Vec::new();
    let mut rest = json.as_str();
    while let Some(start) = rest.find("\"") {
        let after = &rest[start + 1..];
        let end = after.find('"').unwrap();
        let key = &after[..end];
        if after[end + 1..].starts_with(':') {
            keys.push(key.to_string());
        }
        rest = &after[end + 1..];
    }
    assert!(keys.len() > 20, "{keys:?}");
    let mut required = 0;
    for key in &keys {
        let pattern = format!("\"{key}\":");
        let at = json.find(&pattern).unwrap();
        // Drop `"key":value` up to the next comma or closing brace at the same depth.
        let tail = &json[at + pattern.len()..];
        let mut depth = 0i32;
        let mut cut = tail.len();
        for (i, ch) in tail.char_indices() {
            match ch {
                '{' | '[' => depth += 1,
                '}' | ']' if depth == 0 => {
                    cut = i;
                    break;
                }
                '}' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    cut = i + 1;
                    break;
                }
                _ => {}
            }
        }
        let mut bad = format!("{}{}", &json[..at], &tail[cut..]);
        bad = bad.replace(",}", "}");
        match MachineImage::from_json(&bad) {
            Ok(_) => {}
            Err(err) => {
                assert_eq!(err, SnapshotError::MissingField(key.clone()), "{key}");
                required += 1;
            }
        }
    }
    assert!(required > 20);
}

#[test]
fn wrong_types_and_ranges_are_typed() {
    let json = MachineImage::capture(&Cpu::new(FlatMemory::new())).to_json();
    let a = json.find("\"a\":").unwrap();
    let end = a + json[a..].find(',').unwrap();
    let with = |v: &str| format!("{}\"a\":{v}{}", &json[..a], &json[end..]);
    assert_eq!(
        MachineImage::from_json(&with("256")).unwrap_err(),
        SnapshotError::OutOfRange {
            kind: "byte",
            value: 256
        }
    );
    assert_eq!(
        MachineImage::from_json(&with("true")).unwrap_err(),
        SnapshotError::WrongType("number")
    );
    assert!(MachineImage::from_json("[]").is_err());
    assert_eq!(
        SnapshotError::BadHex(b'z').to_string(),
        "hex digit 'z' is not hexadecimal"
    );
    let as_string: String = SnapshotError::UnsupportedVersion(3).into();
    assert_eq!(as_string, "unsupported snapshot version 3");
}
