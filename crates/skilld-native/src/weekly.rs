//! Reads and writes the weekly sign-in notice state.
//!
//! The decision lives in `skilld_command::weekly`. This file only touches disk.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use skilld_command::weekly::WeeklyNoticeState;

const STATE_FILE: &str = "weekly-notice.json";

/// The stored state, or a default when nothing is stored yet.
///
/// A missing or corrupt file is not an error worth reporting. The only cost is
/// one extra notice, so it returns the default and lets the caller continue.
pub fn read_state(data_root: &Path) -> WeeklyNoticeState {
    fs::read(data_root.join(STATE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn write_state(data_root: &Path, state: &WeeklyNoticeState) -> io::Result<()> {
    fs::create_dir_all(data_root)?;
    let bytes = serde_json::to_vec(state).map_err(io::Error::other)?;
    let mut file = tempfile::NamedTempFile::new_in(data_root)?;
    file.write_all(&bytes)?;
    file.persist(data_root.join(STATE_FILE))
        .map(drop)
        .map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_reads_as_the_default() {
        let root = tempfile::tempdir().expect("temp dir");
        assert_eq!(read_state(root.path()), WeeklyNoticeState::default());
    }

    #[test]
    fn a_written_state_reads_back() {
        let root = tempfile::tempdir().expect("temp dir");
        let state = WeeklyNoticeState {
            shown_at: 4_200,
            shown_count: 2,
        };
        write_state(root.path(), &state).expect("write");
        assert_eq!(read_state(root.path()), state);
    }

    #[test]
    fn a_corrupt_file_reads_as_the_default() {
        let root = tempfile::tempdir().expect("temp dir");
        fs::write(root.path().join(STATE_FILE), b"not json").expect("write");
        assert_eq!(read_state(root.path()), WeeklyNoticeState::default());
    }
}
