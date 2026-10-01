use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

pub fn create_temp_dir(name: &str) -> PathBuf {
    loop {
        // Clock precision cannot guarantee uniqueness between concurrent tests.
        let id = NEXT_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!("{name}_{}_{id}", std::process::id()));
        match fs::create_dir(&directory) {
            Ok(()) => return directory,
            // Reserve directories exclusively, including when a previous run left one behind.
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => panic!("Unable to create {}: {error}", directory.display()),
        }
    }
}
