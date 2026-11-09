use std::io::{BufRead, BufReader, Read, Write};

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&String> {
        let target = name.to_ascii_lowercase();
        self.headers.iter().find_map(|(n, v)| {
            if n.eq_ignore_ascii_case(&target) {
                Some(v)
            } else {
                None
            }
        })
    }

    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub fn read_request(stream: &mut std::net::TcpStream) -> Result<Request, String> {
    let cloned = stream
        .try_clone()
        .map_err(|e| format!("socket clone: {e}"))?;
    let mut reader = BufReader::new(cloned);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut split = line.split_whitespace();
    let method = split.next().ok_or("empty line")?.to_string();
    let path = split.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    let mut content_len: usize = 0usize;
    loop {
        line.clear();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.trim().split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_len = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; content_len];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(Request {
        method,
        path,
        body,
        headers,
    })
}

pub fn write_json(
    stream: &mut std::net::TcpStream,
    status: u16,
    body: &serde_json::Value,
) -> Result<(), String> {
    let text = serde_json::to_string(body).map_err(|e| e.to_string())?;
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
        text.len()
    );
    stream
        .write_all(head.as_bytes())
        .map_err(|e| e.to_string())?;
    stream
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}
