//! Loopback redirect server. NEROA_OAUTH_LOOPBACK_V11.
//!
//! Binds 127.0.0.1 on an ephemeral port, accepts exactly one request - the
//! provider's redirect - parses the query, and returns a small page telling
//! the user to go back to Neroa. Deliberately hand-rolled over a TcpStream:
//! it handles one GET and nothing else, so a full HTTP server would be more
//! surface than the job needs.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use super::OAuthError;

pub struct LoopbackServer {
    listener: TcpListener,
    port: u16,
}

/// Parsed redirect query.
#[derive(Debug, Default)]
pub struct Redirect {
    pub code: Option<String>,
    pub state: String,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

impl LoopbackServer {
    pub fn bind() -> Result<Self, OAuthError> {
        // Port 0 asks the OS for a free ephemeral port, which is what RFC 8252
        // recommends over a fixed one.
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        Ok(Self { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Block until the redirect arrives or the deadline passes.
    pub async fn wait_for_redirect(self, timeout: Duration) -> Result<Redirect, OAuthError> {
        let deadline = Instant::now() + timeout;

        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    return handle(stream);
                }

                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(OAuthError::Timeout);
                    }

                    tokio::time::sleep(Duration::from_millis(50)).await;
                }

                Err(error) => return Err(OAuthError::Io(error)),
            }
        }
    }
}

fn handle(mut stream: TcpStream) -> Result<Redirect, OAuthError> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let mut buffer = [0u8; 8192];
    let read = stream.read(&mut buffer).unwrap_or(0);
    let request = String::from_utf8_lossy(&buffer[..read]);

    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let redirect = parse_query(target);

    let body = if redirect.error.is_some() {
        "Sign-in failed. You can close this tab and return to Neroa."
    } else if redirect.code.is_some() {
        "Signed in. You can close this tab and return to Neroa."
    } else {
        "Unexpected redirect. Return to Neroa and try again."
    };

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n\
         <!doctype html><meta charset=utf-8><title>Neroa</title>\
         <body style=\"font:16px system-ui;display:grid;place-items:center;height:100vh;margin:0;\
         background:#0b1220;color:#e8eefc\"><p>{}</p></body>",
        body.len() + 200,
        body,
    );

    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    Ok(redirect)
}

fn parse_query(target: &str) -> Redirect {
    let mut redirect = Redirect::default();

    let Some(query) = target.split('?').nth(1) else {
        return redirect;
    };

    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = decode(parts.next().unwrap_or(""));

        match key {
            "code" => redirect.code = Some(value),
            "state" => redirect.state = value,
            "error" => redirect.error = Some(value),
            "error_description" => redirect.error_description = Some(value),
            _ => {}
        }
    }

    redirect
}

/// Minimal percent-decoding for query values.
fn decode(value: &str) -> String {
    let bytes = value.replace('+', " ");
    let bytes = bytes.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_and_state() {
        let r = parse_query("/?code=abc123&state=xyz&scope=email%20profile");
        assert_eq!(r.code.as_deref(), Some("abc123"));
        assert_eq!(r.state, "xyz");
    }

    #[test]
    fn parses_provider_error() {
        let r = parse_query("/?error=access_denied&error_description=User%20said%20no&state=s");
        assert_eq!(r.error.as_deref(), Some("access_denied"));
        assert_eq!(r.error_description.as_deref(), Some("User said no"));
    }
}
