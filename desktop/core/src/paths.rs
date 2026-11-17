use std::path::PathBuf;

/// Application data directory: `<data_dir>/cz.strazce-katastru.app`.
/// The only place that computes this path.
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cz.strazce-katastru.app")
}

pub fn db_path() -> PathBuf {
    data_dir().join("strazce.db")
}

pub fn lock_path() -> PathBuf {
    data_dir().join("strazce.lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_live_in_data_dir() {
        assert!(data_dir().ends_with("cz.strazce-katastru.app"));
        assert_eq!(db_path(), data_dir().join("strazce.db"));
        assert_eq!(lock_path(), data_dir().join("strazce.lock"));
    }
}
