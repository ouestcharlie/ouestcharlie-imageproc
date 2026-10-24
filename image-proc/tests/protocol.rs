//! process_line: JSON parsing and protocol version checks.

use image_proc::process_line;
use image_proc::protocol::Response;

fn error_msg(r: Response) -> String {
    match r {
        Response::Error { error } => error,
        _ => panic!("expected Error response, got something else"),
    }
}

#[test]
fn test_missing_protocol_version() {
    let line = r#"{"photo": {"path": "/tmp/x.jpg", "ext": ".jpg", "content_hash": "abc"}, "max_long_edge": 1440, "quality": 85, "output": "/tmp/out.jpg"}"#;
    let err = error_msg(process_line(line, 1));
    assert!(
        err.contains("missing or invalid protocol_version"),
        "got: {err}"
    );
}

#[test]
fn test_wrong_major_version() {
    let line = r#"{"protocol_version": 2, "photo": {"path": "/tmp/x.jpg", "ext": ".jpg", "content_hash": "abc"}, "max_long_edge": 1440, "quality": 85, "output": "/tmp/out.jpg"}"#;
    let err = error_msg(process_line(line, 1));
    assert!(
        err.contains("unsupported protocol version 2, expected 1"),
        "got: {err}"
    );
}

#[test]
fn test_version_zero_rejected() {
    let line = r#"{"protocol_version": 0, "photo": {"path": "/tmp/x.jpg", "ext": ".jpg", "content_hash": "abc"}, "max_long_edge": 1440, "quality": 85, "output": "/tmp/out.jpg"}"#;
    let err = error_msg(process_line(line, 1));
    assert!(
        err.contains("unsupported protocol version 0, expected 1"),
        "got: {err}"
    );
}

#[test]
fn test_protocol_version_string_rejected() {
    // protocol_version must be an integer, not a string.
    let line = r#"{"protocol_version": "1", "photo": {"path": "/tmp/x.jpg", "ext": ".jpg", "content_hash": "abc"}, "max_long_edge": 1440, "quality": 85, "output": "/tmp/out.jpg"}"#;
    let err = error_msg(process_line(line, 1));
    assert!(
        err.contains("missing or invalid protocol_version"),
        "got: {err}"
    );
}

#[test]
fn test_invalid_json() {
    let err = error_msg(process_line("not json", 1));
    assert!(err.contains("invalid JSON input"), "got: {err}");
}

#[test]
fn test_correct_version_dispatches_to_invalid_request() {
    // Correct protocol_version but unrecognised shape → invalid request, not version error.
    let line = r#"{"protocol_version": 1, "unknown_command": true}"#;
    let err = error_msg(process_line(line, 1));
    assert!(err.contains("invalid request"), "got: {err}");
}
