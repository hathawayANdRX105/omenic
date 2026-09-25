use std::io::{self, BufRead, Write};

/// Write a JSON line to a writer and flush.
///
/// Expects the line to already include the newline (as produced by
/// `wire::jsonrpc::encode_line`). Appends nothing additional.
pub fn write_json_line<W: Write>(writer: &mut W, line: &str) -> io::Result<()> {
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// Read a line from a reader into a buffer.
///
/// Returns:
/// - Ok(true) on successful read (buffer contains line without newline)
/// - Ok(false) on EOF (buffer unchanged)
/// - Err(_) on I/O error (buffer may be partially filled)
pub fn read_line<R: BufRead>(reader: &mut R, buf: &mut String) -> io::Result<bool> {
    buf.clear();
    let n = reader.read_line(buf)?;
    if n == 0 {
        return Ok(false);
    }
    if buf.ends_with('\n') {
        buf.truncate(buf.len() - 1);
    }
    Ok(true)
}
