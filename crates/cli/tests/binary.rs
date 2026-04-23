//! End-to-end smoke test — spawn the compiled binary, hit it with a
//! JSON-RPC call, verify the reply. Proves that clap + the server
//! crate wire together end-to-end and that `--help` runs without
//! panicking.

use std::process::Stdio;
use std::time::Duration;

fn binary_path() -> std::path::PathBuf {
    // Cargo sets CARGO_BIN_EXE_<bin_name> for every binary in the
    // crate under test. `tidepool-rpc` → CARGO_BIN_EXE_tidepool-rpc.
    env!("CARGO_BIN_EXE_tidepool-rpc").into()
}

#[test]
fn help_runs_and_mentions_start_subcommand() {
    let output = std::process::Command::new(binary_path())
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn");
    assert!(output.status.success(), "exit status: {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("start"),
        "--help output should list the start subcommand; got:\n{stdout}"
    );
}

#[test]
fn version_runs() {
    let output = std::process::Command::new(binary_path())
        .arg("--version")
        .output()
        .expect("spawn");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tidepool-rpc"));
}

#[tokio::test]
async fn start_binary_serves_tidepool_info() {
    // Pick a free port by binding briefly.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    // Use a nonsense upstream — we only exercise the native dispatch
    // path (tidepool_info), which doesn't touch the upstream.
    let mut child = std::process::Command::new(binary_path())
        .args([
            "start",
            "--port",
            &port.to_string(),
            "--upstream",
            "http://127.0.0.1:1",
            "--rpc-timeout-ms",
            "1000",
        ])
        .env("RUST_LOG", "error") // quiet the test output
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn binary");

    // Poll for the server to start listening instead of sleeping a
    // fixed duration — CI machines are sometimes slow.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("binary didn't start listening within 10s");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let resp: serde_json::Value = client
        .post(format!("http://127.0.0.1:{port}"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tidepool_info",
            "params": {}
        }))
        .send()
        .await
        .expect("post")
        .json()
        .await
        .expect("json");

    // Always tear the child down even if asserts fail below.
    let _ = child.kill();
    let _ = child.wait();

    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["name"], "tidepool-rpc");
}
