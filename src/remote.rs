use std::path::Path;
use std::time::{Duration, Instant};

pub fn post_form(remote: Option<&str>, fields: &[(&str, &[u8])]) {
    let Some(remote) = remote.filter(|value| !value.is_empty()) else {
        return;
    };
    report(send_request(
        remote,
        "application/x-www-form-urlencoded",
        &encode_form(fields),
    ));
}

pub fn post_image(remote: Option<&str>, id: &[u8], filename: &str, alt: &[u8], image_path: &Path) {
    let Some(remote) = remote.filter(|value| !value.is_empty()) else {
        return;
    };
    let image = match std::fs::read(image_path) {
        Ok(image) => image,
        Err(error) => {
            report(Err(format!("open {}: {error}", image_path.display())));
            return;
        }
    };
    let boundary = uuid::Uuid::new_v4().simple().to_string();
    let body = image_body(&boundary, id, filename, alt, image_path, &image);
    report(send_request(
        remote,
        &format!("multipart/form-data; boundary={boundary}"),
        &body,
    ));
}

fn report(result: Result<u16, String>) {
    match result {
        Ok(status) if status >= 400 => {
            eprintln!("mdrun: remote POST warning: server returned {status}");
        }
        Err(error) => eprintln!("mdrun: remote POST warning: {error}"),
        Ok(_) => {}
    }
}

fn send_request(remote: &str, content_type: &str, body: &[u8]) -> Result<u16, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut url = url::Url::parse(remote).map_err(|error| error.to_string())?;
    url.set_fragment(None);
    let mut post = true;
    let mut referer = None::<String>;
    let mut requests = 0;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| "request timed out".to_owned())?;
        let response = if post {
            let mut request = agent
                .post(url.as_str())
                .header("Content-Type", content_type);
            if let Some(referer) = &referer {
                request = request.header("Referer", referer.as_str());
            }
            request
                .config()
                .timeout_global(Some(remaining))
                .build()
                .send(body)
        } else {
            let mut request = agent.get(url.as_str());
            if let Some(referer) = &referer {
                request = request.header("Referer", referer.as_str());
            }
            request
                .config()
                .timeout_global(Some(remaining))
                .build()
                .call()
        }
        .map_err(|error| error.to_string())?;
        requests += 1;
        let status = response.status().as_u16();
        if !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Ok(status);
        }
        let Some(location) = response.headers().get("Location") else {
            return Ok(status);
        };
        let location = location.to_str().map_err(|error| error.to_string())?;
        if location.is_empty() {
            return Ok(status);
        }
        let mut next = url.join(location).map_err(|error| error.to_string())?;
        next.set_fragment(None);
        // Go's net/http client replays buffered POST bodies for 307/308; ureq 3.5.0
        // rejects them: https://docs.rs/ureq/3.5.0/ureq/enum.Error.html#variant.RedirectFailed.
        if requests >= 10 {
            return Err("stopped after 10 redirects".to_owned());
        }
        if matches!(status, 301..=303) {
            post = false;
        }
        referer = if url.scheme() == "https" && next.scheme() != "https" {
            None
        } else {
            let mut previous = url.clone();
            let _ = previous.set_username("");
            let _ = previous.set_password(None);
            Some(previous.into())
        };
        url = next;
    }
}

fn encode_form(fields: &[(&str, &[u8])]) -> Vec<u8> {
    fn append_encoded(output: &mut Vec<u8>, value: &[u8]) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        for &byte in value {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    output.push(byte);
                }
                b' ' => output.push(b'+'),
                _ => output.extend_from_slice(&[
                    b'%',
                    HEX[usize::from(byte >> 4)],
                    HEX[usize::from(byte & 15)],
                ]),
            }
        }
    }

    let mut fields = fields.to_vec();
    fields.sort_by_key(|&(name, _)| name);
    let mut body = Vec::new();
    for (index, (name, value)) in fields.into_iter().enumerate() {
        if index > 0 {
            body.push(b'&');
        }
        append_encoded(&mut body, name.as_bytes());
        body.push(b'=');
        append_encoded(&mut body, value);
    }
    body
}

fn image_body(
    boundary: &str,
    id: &[u8],
    filename: &str,
    alt: &[u8],
    image_path: &Path,
    image: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in [
        ("uuid", id),
        ("command", b"image".as_slice()),
        ("filename", filename.as_bytes()),
        ("alt", alt),
    ] {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"")
            .as_bytes(),
    );
    for &byte in image_path.as_os_str().as_encoded_bytes() {
        if matches!(byte, b'"' | b'\\') {
            body.push(b'\\');
        }
        body.push(byte);
    }
    body.extend_from_slice(b"\"\r\nContent-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(image);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    type CapturedRequest = (String, Vec<u8>);

    #[test]
    fn redirects_preserve_post_bytes_only_for_307_and_308() {
        for status in [301, 302, 303, 307, 308] {
            let (remote, capture) = capture_responses(vec![
                format!("HTTP/1.1 {status} Redirect\r\nLocation: ../final\r\n"),
                "HTTP/1.1 503 Service Unavailable\r\n".to_owned(),
            ]);
            let result = send_request(
                &remote,
                "application/x-www-form-urlencoded",
                b"output=%FF%00",
            );
            let requests = capture.join().unwrap();
            assert_eq!(result.unwrap(), 503);
            let repeated_body = status >= 307;
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].0, "POST /nested/start HTTP/1.1\r\n");
            assert_eq!(requests[0].1, b"output=%FF%00");
            assert_eq!(
                requests[1].0,
                if repeated_body {
                    "POST /final HTTP/1.1\r\n"
                } else {
                    "GET /final HTTP/1.1\r\n"
                }
            );
            assert_eq!(
                requests[1].1,
                if repeated_body {
                    b"output=%FF%00".as_slice()
                } else {
                    b"".as_slice()
                }
            );
        }
    }

    #[test]
    fn redirect_loops_stop_after_ten_requests() {
        let (remote, capture) = capture_responses(vec![
            "HTTP/1.1 302 Redirect\r\nLocation: /again\r\n".to_owned();
            10
        ]);
        let result = send_request(
            &remote,
            "application/x-www-form-urlencoded",
            b"command=note",
        );
        let requests = capture.join().unwrap();
        assert_eq!(result.unwrap_err(), "stopped after 10 redirects");
        assert_eq!(requests.len(), 10);
    }

    fn capture_responses(
        responses: Vec<String>,
    ) -> (String, thread::JoinHandle<Vec<CapturedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let remote = format!("http://{}/nested/start", listener.local_addr().unwrap());
        let capture = thread::spawn(move || {
            let mut requests = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(2);
            for response in responses {
                let (stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if Instant::now() >= deadline {
                                return requests;
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("accept request: {error}"),
                    }
                };
                // macOS accept inherits listener properties; request parsing needs blocking reads.
                // https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/accept.2.html
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    assert!(reader.read_line(&mut header).unwrap() > 0);
                    if header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                requests.push((request_line, body));
                write!(
                    reader.get_mut(),
                    "{response}Content-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
            requests
        });
        (remote, capture)
    }

    #[test]
    fn form_encoding_sorts_fields_and_preserves_non_utf8_bytes() {
        assert_eq!(
            encode_form(&[
                ("uuid", b"legacy-fixed"),
                ("input", b"a b+c&=\0\xff~*"),
                ("command", b"exec"),
            ]),
            b"command=exec&input=a+b%2Bc%26%3D%00%FF~%2A&uuid=legacy-fixed"
        );
        assert_eq!(
            encode_form(&[("a b", b""), ("a b", b"second")]),
            b"a+b=&a+b=second"
        );
    }

    #[test]
    fn image_multipart_preserves_binary_fields_upload_path_and_content() {
        let expected = concat!(
            "--fixed\r\nContent-Disposition: form-data; name=\"uuid\"\r\n\r\nlegacy-fixed\r\n",
            "--fixed\r\nContent-Disposition: form-data; name=\"command\"\r\n\r\nimage\r\n",
            "--fixed\r\nContent-Disposition: form-data; name=\"filename\"\r\n\r\ncopy.png\r\n",
            "--fixed\r\nContent-Disposition: form-data; name=\"alt\"\r\n\r\n"
        );
        let mut expected = expected.as_bytes().to_vec();
        expected.extend_from_slice(b"caption\0\xff\r\n--fixed\r\nContent-Disposition: form-data; name=\"image\"; filename=\"nested/a\\\"b\\\\c.png\"\r\nContent-Type: application/octet-stream\r\n\r\n\x89PNG\0\xff\r\n--fixed--\r\n");
        assert_eq!(
            image_body(
                "fixed",
                b"legacy-fixed",
                "copy.png",
                b"caption\0\xff",
                Path::new("nested/a\"b\\c.png"),
                b"\x89PNG\0\xff",
            ),
            expected
        );
    }
}
