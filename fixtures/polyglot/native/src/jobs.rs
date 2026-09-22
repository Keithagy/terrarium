pub struct Job {
    pub id: u32,
}

impl Job {
    pub fn fetch_all() -> Vec<Job> {
        let _body = reqwest::blocking::get("http://localhost:8080/api/jobs");
        vec![]
    }
}

pub fn sync_jobs() {
    let _jobs = Job::fetch_all();
}
