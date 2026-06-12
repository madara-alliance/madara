use mc_class_exec::config::NativeConfig;
use mc_db::{rocksdb::RocksDBConfig, MadaraBackend, MadaraBackendConfig};
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use mp_chain_config::ChainConfig;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, ErrorKind, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const SERVER_TIMEOUT: Duration = Duration::from_secs(10);

fn create_source_db(base_path: &Path) {
    let backend = MadaraBackend::open_rocksdb(
        base_path,
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig::default(),
        RocksDBConfig::default(),
        Arc::new(NativeConfig::default()),
    )
    .expect("source backend should open");

    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .expect("source block should be written");
    backend.flush().expect("source backend should flush");
}

fn default_manifest_path(snapshot_path: &Path) -> PathBuf {
    let mut path = snapshot_path.as_os_str().to_os_string();
    path.push(".manifest.json");
    PathBuf::from(path)
}

fn madara_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_madara"));
    command
        .env("RUST_LOG", "error")
        .env_remove("MADARA_CONFIG_FILE")
        .env_remove("MADARA_FULL")
        .env_remove("MADARA_SEQUENCER")
        .env_remove("MADARA_DEVNET")
        .env_remove("MADARA_NETWORK")
        .env_remove("MADARA_PRESET")
        .env_remove("MADARA_CHAIN_CONFIG_PATH")
        .env_remove("MADARA_CHAIN_CONFIG_OVERRIDE")
        .env_remove("MADARA_BASE_PATH")
        .env_remove("MADARA_BOOTSTRAP_SNAPSHOT")
        .env_remove("MADARA_BOOTSTRAP_SNAPSHOT_URL")
        .env_remove("MADARA_BOOTSTRAP_SNAPSHOT_MANIFEST")
        .env_remove("MADARA_BOOTSTRAP_SNAPSHOT_MANIFEST_URL")
        .env_remove("MADARA_CREATE_BOOTSTRAP_SNAPSHOT")
        .env_remove("MADARA_CREATE_BOOTSTRAP_SNAPSHOT_MANIFEST");
    command
}

fn output_text(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_success(output: Output) {
    assert!(output.status.success(), "{}", output_text(&output));
}

fn assert_failure_contains(output: Output, expected: &str) {
    assert!(!output.status.success(), "command unexpectedly succeeded:\n{}", output_text(&output));
    assert!(
        output_text(&output).contains(expected),
        "expected output to contain `{expected}`:\n{}",
        output_text(&output)
    );
}

fn command_output(command: &mut Command) -> Output {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("madara command should spawn");
    let deadline = Instant::now() + COMMAND_TIMEOUT;

    loop {
        if child.try_wait().expect("madara command status should be readable").is_some() {
            return child.wait_with_output().expect("madara command output should be readable");
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().expect("timed out madara command output should be readable");
            panic!("madara command timed out after {:?}\n{}", COMMAND_TIMEOUT, output_text(&output));
        }

        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn madara_binary_smokes_bootstrap_snapshot_create_and_import_guards() {
    let temp = tempfile::TempDir::new().unwrap();
    let source_base_path = temp.path().join("source");
    let archive_path = temp.path().join("snapshot.tar.gz");
    let manifest_path = default_manifest_path(&archive_path);
    create_source_db(&source_base_path);

    let mut create_command = madara_command();
    create_command.args([
        "--name",
        "bootstrap-smoke",
        "--full",
        "--network",
        "devnet",
        "--base-path",
        source_base_path.to_str().unwrap(),
        "--create-bootstrap-snapshot",
        archive_path.to_str().unwrap(),
    ]);
    let create_output = command_output(&mut create_command);
    assert_success(create_output);

    assert!(archive_path.is_file(), "snapshot archive should exist");
    assert!(manifest_path.is_file(), "snapshot manifest should exist");

    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).expect("manifest should be JSON");
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(manifest["chain_id"], "MADARA_DEVNET");
    assert_eq!(manifest["block_number"], 0);
    assert!(manifest["archive_sha256"].as_str().unwrap().len() == 64);
    assert!(manifest["archive_size_bytes"].as_u64().unwrap() > 0);

    let wrong_chain_base_path = temp.path().join("wrong-chain-target");
    let mut wrong_chain_command = madara_command();
    wrong_chain_command.args([
        "--name",
        "bootstrap-smoke",
        "--full",
        "--network",
        "sepolia",
        "--base-path",
        wrong_chain_base_path.to_str().unwrap(),
        "--bootstrap-snapshot",
        archive_path.to_str().unwrap(),
    ]);
    let wrong_chain_output = command_output(&mut wrong_chain_command);
    assert_failure_contains(wrong_chain_output, "snapshot is for chain id `MADARA_DEVNET`");

    let (server_url, server_handle) = serve_files(vec![("/snapshot.tar.gz.manifest.json", manifest_path.clone())]);
    let remote_wrong_chain_base_path = temp.path().join("remote-wrong-chain-target");
    let mut remote_wrong_chain_command = madara_command();
    remote_wrong_chain_command.args([
        "--name",
        "bootstrap-smoke",
        "--full",
        "--network",
        "sepolia",
        "--base-path",
        remote_wrong_chain_base_path.to_str().unwrap(),
        "--bootstrap-snapshot-url",
        server_url.join("snapshot.tar.gz").unwrap().as_str(),
    ]);
    let remote_wrong_chain_output = command_output(&mut remote_wrong_chain_command);
    assert_failure_contains(remote_wrong_chain_output, "snapshot is for chain id `MADARA_DEVNET`");
    server_handle.join().unwrap();

    let non_empty_base_path = temp.path().join("non-empty-target");
    std::fs::create_dir_all(&non_empty_base_path).unwrap();
    std::fs::write(non_empty_base_path.join("existing"), b"data").unwrap();
    let mut non_empty_command = madara_command();
    non_empty_command.args([
        "--name",
        "bootstrap-smoke",
        "--full",
        "--network",
        "devnet",
        "--base-path",
        non_empty_base_path.to_str().unwrap(),
        "--bootstrap-snapshot",
        archive_path.to_str().unwrap(),
    ]);
    let non_empty_output = command_output(&mut non_empty_command);
    assert_failure_contains(non_empty_output, "is not empty");
}

fn serve_files(files: Vec<(&'static str, PathBuf)>) -> (url::Url, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = url::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let request_count = files.len();
    let files = files.into_iter().map(|(path, file)| (path.to_string(), file)).collect::<HashMap<_, _>>();

    let handle = thread::spawn(move || {
        let deadline = Instant::now() + SERVER_TIMEOUT;
        let mut served = 0;

        while served < request_count {
            match listener.accept() {
                Ok((stream, _)) => {
                    serve_connection(stream, &files);
                    served += 1;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        panic!("test HTTP server timed out after serving {served}/{request_count} requests");
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("test HTTP server failed to accept connection: {err}"),
            }
        }
    });

    (base_url, handle)
}

fn serve_connection(mut stream: TcpStream, files: &HashMap<String, PathBuf>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let path = request_line.split_whitespace().nth(1).unwrap();

    if let Some(file_path) = files.get(path) {
        let body = std::fs::read(file_path).unwrap();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        stream.write_all(&body).unwrap();
    } else {
        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    }
}
