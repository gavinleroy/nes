use std::io::{Read, Write};

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn param(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

pub fn parse_target(target: &str) -> (String, Vec<(String, String)>) {
    match target.split_once('?') {
        None => (target.to_string(), Vec::new()),
        Some((path, qs)) => {
            let query = qs
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| match p.split_once('=') {
                    Some((k, v)) => (k.to_string(), v.to_string()),
                    None => (p.to_string(), String::new()),
                })
                .collect();
            (path.to_string(), query)
        }
    }
}

pub fn parse_head(head: &str) -> Option<(String, String, usize)> {
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_string();
    let target = request_line.next()?.to_string();
    let mut content_length = 0;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok()?;
            }
        }
    }
    Some((method, target, content_length))
}

pub fn read_request(stream: &mut impl Read) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() > 1 << 20 {
            return None;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&buf[..header_end]).ok()?.to_string();
    let (method, target, content_length) = parse_head(&head)?;
    let mut body: Vec<u8> = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);
    let (path, query) = parse_target(&target);
    Some(Request {
        method,
        path,
        query,
        body,
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

pub fn respond(stream: &mut impl Write, status: u16, content_type: &str, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_parsing_all_shapes() {
        for (target, path, params) in [
            ("/", "/", vec![]),
            ("/step", "/step", vec![]),
            ("/step?", "/step", vec![]),
            ("/step?a=1", "/step", vec![("a", "1")]),
            ("/step?a=1&b=2", "/step", vec![("a", "1"), ("b", "2")]),
            ("/step?b=2&a=1", "/step", vec![("b", "2"), ("a", "1")]),
            ("/step?flag", "/step", vec![("flag", "")]),
            ("/step?a=1&&b=2", "/step", vec![("a", "1"), ("b", "2")]),
            ("/x?a=1=2", "/x", vec![("a", "1=2")]),
        ] {
            let (p, q) = parse_target(target);
            assert_eq!(p, path);
            let want: Vec<(String, String)> = params
                .into_iter()
                .map(|(k, v): (&str, &str)| (k.to_string(), v.to_string()))
                .collect();
            assert_eq!(q, want);
        }
    }

    #[test]
    fn head_parsing_content_length_case_insensitive() {
        for name in [
            "Content-Length",
            "content-length",
            "CONTENT-LENGTH",
            "CoNtEnT-lEnGtH",
        ] {
            let head = format!("POST /rom HTTP/1.1\r\nHost: x\r\n{name}: 42");
            let (method, target, len) = parse_head(&head).unwrap();
            assert_eq!(method, "POST");
            assert_eq!(target, "/rom");
            assert_eq!(len, 42);
        }
        let (_, _, len) = parse_head("GET / HTTP/1.1\r\nHost: x").unwrap();
        assert_eq!(len, 0);
    }

    #[test]
    fn request_roundtrip_over_reader() {
        for body_len in [0usize, 1, 100, 5000, 70000] {
            let body: Vec<u8> = (0..body_len).map(|i| (i * 7 % 256) as u8).collect();
            let mut raw = format!("POST /rom?x=1 HTTP/1.1\r\nContent-Length: {body_len}\r\n\r\n")
                .into_bytes();
            raw.extend_from_slice(&body);
            let mut cursor = std::io::Cursor::new(raw);
            let req = read_request(&mut cursor).unwrap();
            assert_eq!(req.method, "POST");
            assert_eq!(req.path, "/rom");
            assert_eq!(req.param("x"), Some("1"));
            assert_eq!(req.body, body);
        }
    }
}
