mod commands;
mod jobs;

pub use commands::scan_repo;

pub fn run() {
    let home = std::env::var("HOME").unwrap_or_default();
    println!("{home}");
    jobs::sync_jobs();
}
