//! Reading a transcript by byte offset: whole new lines after a point, or
//! the one line at a point.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Folds the complete lines after `offset` into the builder and returns the
/// offset of the first line not yet complete.
pub(super) fn read_from(
    path: &Path,
    offset: u64,
    mut push: impl FnMut(u64, &str),
) -> anyhow::Result<u64> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    // A shorter file than we have read is a rewritten one: start over.
    let offset = if len < offset { 0 } else { offset };
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut start = 0usize;
    while let Some(end) = bytes[start..].iter().position(|&b| b == b'\n') {
        let line = &bytes[start..start + end];
        if let Ok(text) = std::str::from_utf8(line) {
            push(offset + start as u64, text);
        }
        start += end + 1;
    }
    Ok(offset + start as u64)
}

pub(super) fn line_at(path: &Path, offset: u64) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line)?;
    Ok(line)
}
