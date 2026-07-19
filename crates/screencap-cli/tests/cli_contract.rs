#![cfg(windows)]
//! End-to-end contract tests for the `screencap-cli` binary.
//!
//! Spawns the real exe and locks the JSON/exit-code surface embedding hosts depend on.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_screencap-cli");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to spawn screencap-cli")
}

fn unique_out() -> PathBuf {
    std::env::temp_dir().join(format!("screencap-smoke-{}.png", std::process::id()))
}

#[test]
fn version_prints_name_and_cargo_version() {
    let out = run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout,
        format!("screencap-cli {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn list_windows_json_contract() {
    let out = run(&["list", "windows", "--json", "--no-log"]);
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], Value::Bool(true));
    assert_eq!(v["command"], Value::String("list windows".to_string()));
    assert!(v["windows"].is_array());
}

#[test]
fn list_monitors_json_contract() {
    let out = run(&["list", "monitors", "--json", "--no-log"]);
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], Value::Bool(true));
    assert_eq!(v["command"], Value::String("list monitors".to_string()));
    assert!(v["monitors"].is_array());
}

#[test]
fn parse_failure_exits_2_without_json() {
    let out_path = unique_out();
    let out = run(&[
        "cap",
        "--method",
        "wgc-window",
        "--foreground",
        "--format",
        "not-a-format",
        "--out",
        out_path.to_str().unwrap(),
        "--no-log",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!out_path.exists());
}

#[test]
fn json_parse_failure_emits_unknown_command_shape() {
    let out_path = unique_out();
    let out = run(&[
        "--json",
        "cap",
        "--method",
        "wgc-window",
        "--foreground",
        "--format",
        "not-a-format",
        "--out",
        out_path.to_str().unwrap(),
        "--no-log",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!out_path.exists());

    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], Value::Bool(false));
    assert_eq!(v["command"], Value::String("unknown".to_string()));

    let message = v["error"]["message"].as_str().unwrap();
    assert!(!message.is_empty());

    let obj = v.as_object().unwrap();
    for key in ["window", "monitor", "crop", "image_stats"] {
        assert!(obj.contains_key(key));
        assert!(obj[key].is_null());
    }
}
