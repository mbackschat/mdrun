use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
const HEADER: &str = "# Remote\n\n<!-- showboat-id: remote-fixed -->\n";
const CODE: &str = "printf 'local effect\\n' > effect; printf 'recorded output\\n'";

type CapturedRequest = (String, Vec<u8>);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mdrun-remote-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("doc.md"), HEADER).unwrap();
        Self(path)
    }

    fn run(&self, remote: &str, code: &str, limit: Duration) -> (Output, Duration) {
        let stdout = fs::File::create(self.0.join("stdout")).unwrap();
        let stderr = fs::File::create(self.0.join("stderr")).unwrap();
        let started = Instant::now();
        let mut child = ChildGuard(Some(
            Command::new(env!("CARGO_BIN_EXE_mdrun"))
                .args(["exec", "doc.md", "sh", code])
                .current_dir(&self.0)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("SHOWBOAT_REMOTE_URL", remote)
                .stdin(Stdio::null())
                .stdout(stdout)
                .stderr(stderr)
                .spawn()
                .unwrap(),
        ));
        let status = loop {
            if let Some(status) = child.0.as_mut().unwrap().try_wait().unwrap() {
                break status;
            }
            assert!(
                started.elapsed() < limit,
                "remote delivery exceeded the {limit:?} process deadline"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let elapsed = started.elapsed();
        child.0.take();
        (
            Output {
                status,
                stdout: fs::read(self.0.join("stdout")).unwrap(),
                stderr: fs::read(self.0.join("stderr")).unwrap(),
            },
            elapsed,
        )
    }

    fn assert_local_effects(&self, output: &Output, code: &str, status: i32) {
        assert_eq!(output.status.code(), Some(status));
        assert_eq!(output.stdout, b"recorded output\n");
        assert_eq!(fs::read(self.0.join("effect")).unwrap(), b"local effect\n");
        assert_eq!(
            fs::read(self.0.join("doc.md")).unwrap(),
            format!("{HEADER}\n```sh\n{code}\n```\n\n```output\nrecorded output\n```\n").as_bytes()
        );
        let warning = String::from_utf8_lossy(&output.stderr);
        assert!(
            warning.starts_with("mdrun: remote POST warning: "),
            "missing remote warning: {warning:?}"
        );
        assert_eq!(warning.lines().count(), 1, "stderr: {warning:?}");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct HeldResponse {
    address: SocketAddr,
    release: Option<Sender<()>>,
    capture: Option<JoinHandle<Option<CapturedRequest>>>,
}

impl HeldResponse {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, released) = mpsc::channel();
        let capture = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            if released.try_recv().is_ok() {
                return None;
            }
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let mut length = None;
            loop {
                let mut header = String::new();
                assert!(reader.read_line(&mut header).unwrap() > 0);
                if header == "\r\n" {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            let mut body = vec![0; length.expect("request content length")];
            reader.read_exact(&mut body).unwrap();
            // Keep the accepted request unanswered until the command has exited.
            let _ = released.recv_timeout(Duration::from_secs(20));
            Some((request, body))
        });
        Self {
            address,
            release: Some(release),
            capture: Some(capture),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/held", self.address)
    }

    fn finish(mut self) -> CapturedRequest {
        self.stop();
        self.capture
            .take()
            .unwrap()
            .join()
            .unwrap()
            .expect("remote request reached the listener")
    }

    fn stop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
            // Wake accept when a broken client never attempted its remote request.
            let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(1));
        }
    }
}

impl Drop for HeldResponse {
    fn drop(&mut self) {
        self.stop();
        if let Some(capture) = self.capture.take() {
            let _ = capture.join();
        }
    }
}

#[test]
fn connection_refusal_warns_without_discarding_local_effects_or_command_status() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let error = TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
    let remote = format!("http://{address}/refused");
    for status in [0, 7] {
        let fixture = Fixture::new();
        let code = format!("{CODE}; exit {status}");
        let (output, _) = fixture.run(&remote, &code, Duration::from_secs(4));
        fixture.assert_local_effects(&output, &code, status);
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .to_ascii_lowercase()
                .contains("connection refused"),
            "stderr: {:?}",
            output.stderr
        );
    }
}

#[test]
fn unanswered_request_times_out_after_ten_seconds_without_discarding_local_effects() {
    let fixture = Fixture::new();
    let server = HeldResponse::new();
    let (output, elapsed) = fixture.run(&server.url(), CODE, Duration::from_secs(15));
    let (request, body) = server.finish();
    fixture.assert_local_effects(&output, CODE, 0);
    assert_eq!(request, "POST /held HTTP/1.1\r\n");
    let body = String::from_utf8(body).unwrap();
    assert!(body.starts_with("command=exec&input="), "body: {body:?}");
    assert!(body.ends_with("&uuid=remote-fixed"), "body: {body:?}");
    assert!(
        elapsed >= Duration::from_secs(9),
        "remote delivery returned before the ten-second timeout: {elapsed:?}"
    );
    let warning = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(
        warning.contains("timed out") || warning.contains("timeout"),
        "stderr: {warning:?}"
    );
}
