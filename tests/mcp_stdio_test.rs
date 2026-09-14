//! End-to-end tests of `docugraph serve`: spawn the real binary and speak JSON-RPC over stdio.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use docugraph::document::model::{Document, DocumentMetadata, Page};
use docugraph::storage::DiskCache;
use serde_json::{Value, json};
use tempfile::tempdir;

/// Generous limit for a single response; a healthy server answers in milliseconds.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

/// A `docugraph serve` child process driven through its stdin and stdout.
struct StdioServer {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout_lines: Receiver<String>,
    stderr: Option<JoinHandle<String>>,
    /// Every line the server wrote to stdout, in order.
    transcript: Vec<String>,
    next_id: u64,
}

impl StdioServer {
    /// Start the server in `cwd` with `DOCUGRAPH_CACHE_DIR=cache_dir` and complete the handshake.
    fn spawn(cwd: &Path, cache_dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_docugraph"))
            .arg("serve")
            .current_dir(cwd)
            .env("DOCUGRAPH_CACHE_DIR", cache_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn docugraph serve");

        let stdout = child.stdout.take().expect("child stdout");
        let (tx, stdout_lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        let mut stderr_pipe = child.stderr.take().expect("child stderr");
        let stderr = thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr_pipe.read_to_string(&mut text);
            text
        });

        let stdin = child.stdin.take();
        let mut server = Self {
            child,
            stdin,
            stdout_lines,
            stderr: Some(stderr),
            transcript: Vec::new(),
            next_id: 0,
        };

        let init = server.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "docugraph-stdio-test", "version": "0" }
            }),
        );
        assert_eq!(
            init["result"]["serverInfo"]["name"], "docugraph-mcp",
            "{init}"
        );
        server.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        server
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        writeln!(stdin, "{message}").expect("write request");
        stdin.flush().expect("flush request");
    }

    /// Send a request and wait for the response with the same id.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));

        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = match self.stdout_lines.recv_timeout(remaining) {
                Ok(line) => line,
                Err(e) => panic!("no response to {method}: {e:?}"),
            };
            let message = parse_frame(&line);
            self.transcript.push(line);
            if message["id"] == id {
                return message;
            }
        }
    }

    /// Call a tool and return its `isError` flag and text content.
    fn call_tool(&mut self, name: &str, arguments: Value) -> (bool, String) {
        let response = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        let result = &response["result"];
        assert!(
            result.is_object(),
            "tools/call {name} must return a tool result, got {response}"
        );
        let text: String = result["content"]
            .as_array()
            .expect("content array")
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect();
        (result["isError"] == true, text)
    }

    /// Close stdin, wait for the server to exit and return what it wrote to stderr.
    fn shutdown(&mut self) -> String {
        drop(self.stdin.take());
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        while self.child.try_wait().expect("poll child").is_none() {
            assert!(
                Instant::now() < deadline,
                "server did not exit after stdin was closed"
            );
            thread::sleep(Duration::from_millis(20));
        }
        loop {
            match self.stdout_lines.recv_timeout(RESPONSE_TIMEOUT) {
                Ok(line) => {
                    parse_frame(&line);
                    self.transcript.push(line);
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => panic!("stdout stayed open after exit"),
            }
        }
        self.stderr
            .take()
            .expect("stderr reader")
            .join()
            .expect("stderr reader thread")
    }
}

impl Drop for StdioServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Every stdout line must be a JSON-RPC 2.0 message: logs and warnings belong on stderr.
fn parse_frame(line: &str) -> Value {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(e) => panic!("stdout line is not JSON ({e}): {line}"),
    };
    assert_eq!(
        message["jsonrpc"], "2.0",
        "stdout line is not JSON-RPC 2.0: {line}"
    );
    message
}

/// Store a small three-page document in `cache_dir`, as `docugraph index` would.
fn write_cached_document(cache_dir: &Path) {
    let mut doc = Document::new(DocumentMetadata {
        id: "stdio-doc".to_string(),
        title: "Stdio Test Manual".to_string(),
        total_pages: 3,
        content_hash: "stdio-test-hash".to_string(),
        ..Default::default()
    });
    doc.add_page(Page::new(1, "Chapter one introduces the stdio transport."));
    doc.add_page(Page::new(2, "Chapter two covers JSON-RPC framing."));
    doc.add_page(Page::new(3, "Chapter three lists the tool errors."));
    DiskCache::new(cache_dir)
        .expect("open cache dir")
        .save(&doc)
        .expect("save document");
}

#[test]
fn stdio_tool_errors_set_is_error_and_stdout_stays_json_rpc() {
    let workdir = tempdir().expect("create workdir");
    let cache = tempdir().expect("create cache dir");
    write_cached_document(cache.path());

    let mut server = StdioServer::spawn(workdir.path(), cache.path());

    let tools = server.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for expected in [
        "document_list",
        "document_info",
        "document_search",
        "document_read_pages",
        "document_read_attachment",
    ] {
        assert!(
            names.contains(&expected),
            "tools/list lacks {expected}: {names:?}"
        );
    }

    let (is_error, text) =
        server.call_tool("document_info", json!({ "document_id": "missing-doc" }));
    assert!(is_error, "an unknown document must set isError: {text}");
    assert!(text.contains("'missing-doc' not found"), "{text}");
    assert!(
        text.contains("'stdio-doc'"),
        "the error should list the ids: {text}"
    );

    let (is_error, text) = server.call_tool(
        "document_search",
        json!({ "query": "json-rpc framing", "document_id": "stdio-dco" }),
    );
    assert!(is_error, "an unknown document_id must set isError: {text}");
    assert!(text.contains("'stdio-doc'"), "{text}");

    let started = Instant::now();
    let (is_error, text) = server.call_tool(
        "document_read_pages",
        json!({ "document_id": "stdio-doc", "page_start": 2, "page_end": u32::MAX }),
    );
    assert!(!is_error, "{text}");
    assert!(text.contains("(pp. 2-3)"), "{text}");
    assert!(text.contains("JSON-RPC framing"), "{text}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "page_end=u32::MAX took {:?}",
        started.elapsed()
    );

    let (is_error, text) = server.call_tool(
        "document_read_pages",
        json!({ "document_id": "stdio-doc", "page_start": 9, "page_end": u32::MAX }),
    );
    assert!(
        is_error,
        "a range past the last page must set isError: {text}"
    );
    assert!(text.contains("has 3 pages"), "{text}");

    let stderr = server.shutdown();
    assert!(!server.transcript.is_empty());
    assert!(
        !stderr.contains("WARNING"),
        "an absolute cache with documents must not warn:\n{stderr}"
    );
}

#[test]
fn stdio_serve_warns_about_a_relative_empty_cache() {
    let workdir = tempdir().expect("create workdir");
    let mut server = StdioServer::spawn(workdir.path(), Path::new("relative-cache"));

    let (is_error, text) = server.call_tool("document_list", json!({}));
    assert!(!is_error, "{text}");
    let resolved = workdir
        .path()
        .canonicalize()
        .expect("canonical workdir")
        .join("relative-cache");
    assert!(
        text.contains(&resolved.display().to_string()),
        "document_list should name the absolute cache dir {}: {text}",
        resolved.display()
    );
    assert!(text.contains("DOCUGRAPH_CACHE_DIR"), "{text}");

    let (is_error, text) = server.call_tool("document_search", json!({ "query": "anything" }));
    assert!(
        is_error,
        "searching an empty cache must set isError: {text}"
    );

    let stderr = server.shutdown();
    assert!(stderr.contains("WARNING"), "{stderr}");
    assert!(stderr.contains("is relative"), "{stderr}");
    assert!(stderr.contains("no indexed documents"), "{stderr}");
}
