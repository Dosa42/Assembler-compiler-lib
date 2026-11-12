//! The LSP "base protocol": each message is a small block of
//! `Header: value\r\n` lines, a blank `\r\n` line, then exactly
//! `Content-Length` bytes of UTF-8-encoded JSON. No other framing exists
//! -- there's no length prefix on the *header* block itself, so it has
//! to be read line-by-line until the blank line is found. This is the
//! one piece of the protocol every LSP client and server, regardless of
//! language, implements identically; see
//! <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#baseProtocol>.

use std::io::{self, BufRead, Write};

/// Reads one framed message body from `input` and returns it as a
/// `String` (the body is required to be valid UTF-8 JSON text).
///
/// Returns `Ok(None)` on a clean EOF before any header bytes are read --
/// the normal way an editor signals "done talking to you" by closing
/// stdin, which should end the server's read loop rather than error out.
/// An EOF *mid-message* (e.g. after `Content-Length` but before all the
/// promised bytes arrive) is a genuine protocol violation and surfaces as
/// an `Err`, since the caller has no complete message to act on.
pub fn read_message<R: BufRead>(input: &mut R) -> io::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    let mut saw_any_header_bytes = false;

    loop {
        let mut line = String::new();
        let n = input.read_line(&mut line)?;
        if n == 0 {
            return if saw_any_header_bytes {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF while reading message headers"))
            } else {
                Ok(None)
            };
        }
        saw_any_header_bytes = true;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // The blank line ends the header block.
        }
        // `Content-Type` (and any other header) is read past and
        // ignored; only `Content-Length` affects framing.
        if let Some(rest) = trimmed.split_once(':') {
            if rest.0.trim().eq_ignore_ascii_case("content-length") {
                content_length = rest.1.trim().parse::<usize>().ok();
            }
        }
    }

    let len = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "message had no Content-Length header"))?;
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    String::from_utf8(buf).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Writes one framed message with `body` as its content. `body` must
/// already be complete, valid JSON text (this function just measures and
/// frames it -- see `json_value::to_string`).
pub fn write_message<W: Write>(output: &mut W, body: &str) -> io::Result<()> {
    let bytes = body.as_bytes();
    // Byte length, not `body.len()` chars/codepoints -- same thing for
    // this crate's `String`s in practice, but the spec is explicit that
    // `Content-Length` counts bytes, so measure the actual bytes we're
    // about to write rather than relying on that coincidence.
    write!(output, "Content-Length: {}\r\n\r\n", bytes.len())?;
    output.write_all(bytes)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    #[test]
    fn reads_a_single_framed_message() {
        let body = "{\"a\":1}";
        let raw = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = BufReader::new(Cursor::new(raw.into_bytes()));
        let msg = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(msg, body);
    }

    #[test]
    fn reads_multiple_back_to_back_messages_from_one_stream() {
        let body_a = "{\"n\":1}";
        let body_b = "{\"n\":2}";
        let raw = format!(
            "Content-Length: {}\r\n\r\n{}Content-Length: {}\r\n\r\n{}",
            body_a.len(),
            body_a,
            body_b.len(),
            body_b
        );
        let mut reader = BufReader::new(Cursor::new(raw.into_bytes()));
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), body_a);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), body_b);
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn ignores_extra_headers_like_content_type() {
        let body = "{\"x\":true}";
        let raw = format!("Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = BufReader::new(Cursor::new(raw.into_bytes()));
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), body);
    }

    #[test]
    fn clean_eof_before_any_message_returns_none() {
        let mut reader = BufReader::new(Cursor::new(Vec::<u8>::new()));
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn eof_mid_message_is_an_error() {
        let mut reader = BufReader::new(Cursor::new(b"Content-Length: 100\r\n\r\nshort".to_vec()));
        assert!(read_message(&mut reader).is_err());
    }

    #[test]
    fn missing_content_length_is_an_error() {
        let mut reader = BufReader::new(Cursor::new(b"Content-Type: text/plain\r\n\r\n".to_vec()));
        assert!(read_message(&mut reader).is_err());
    }

    #[test]
    fn write_then_read_round_trips() {
        let mut buf = Vec::new();
        write_message(&mut buf, "{\"hello\":\"world\"}").unwrap();
        let mut reader = BufReader::new(Cursor::new(buf));
        let msg = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(msg, "{\"hello\":\"world\"}");
    }

    #[test]
    fn content_length_counts_bytes_not_chars_for_multibyte_utf8() {
        // "café" is 4 chars but 5 bytes (é is 2 bytes in UTF-8); a framing
        // implementation that measured chars instead of bytes would
        // truncate this and corrupt the message.
        let body = "{\"note\":\"café\"}";
        let mut buf = Vec::new();
        write_message(&mut buf, body).unwrap();
        let raw = String::from_utf8(buf.clone()).unwrap();
        assert!(raw.starts_with(&format!("Content-Length: {}\r\n\r\n", body.as_bytes().len())));
        let mut reader = BufReader::new(Cursor::new(buf));
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), body);
    }
}
