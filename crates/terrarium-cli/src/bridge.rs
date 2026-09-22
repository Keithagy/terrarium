//! HTTP client for the running app's agent bridge.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 47311;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeFile {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: String,
}

pub fn bridge_file_path() -> PathBuf {
    terrarium_core::cache::home_dir().join("bridge.json")
}

pub fn read_bridge_file() -> Option<BridgeFile> {
    let b = std::fs::read(bridge_file_path()).ok()?;
    serde_json::from_slice(&b).ok()
}

pub struct Bridge {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl Bridge {
    pub fn discover() -> Bridge {
        let file = read_bridge_file();
        let port = std::env::var("TERRARIUM_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .or(file.as_ref().map(|f| f.port))
            .unwrap_or(DEFAULT_PORT);
        let token = std::env::var("TERRARIUM_TOKEN")
            .ok()
            .or(file.map(|f| f.token));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .build()
            .into();
        Bridge {
            base: format!("http://127.0.0.1:{port}"),
            token,
            agent,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        let mut req = self.agent.get(format!("{}{}", self.base, path));
        if let Some(t) = &self.token {
            req = req.header("x-terrarium-token", t);
        }
        let mut res = req.call().map_err(|e| connect_error(e, &self.base))?;
        let status = res.status().as_u16();
        let body: Value = res.body_mut().read_json().unwrap_or(Value::Null);
        if status >= 400 {
            return Err(anyhow!(
                "app returned {status}: {}",
                body.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown error")
            ));
        }
        Ok(body)
    }

    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let mut req = self.agent.get(format!("{}{}", self.base, path));
        if let Some(t) = &self.token {
            req = req.header("x-terrarium-token", t);
        }
        let mut res = req.call().map_err(|e| connect_error(e, &self.base))?;
        if res.status().as_u16() >= 400 {
            return Err(anyhow!("app returned {}", res.status()));
        }
        res.body_mut().read_to_vec().context("reading body")
    }

    pub fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.post_with(&self.agent, path, body)
    }

    /// POST for calls that run for minutes (a Claude design run).
    pub fn post_long(&self, path: &str, body: Value) -> Result<Value> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(1800)))
            .http_status_as_error(false)
            .build()
            .into();
        self.post_with(&agent, path, body)
    }

    fn post_with(&self, agent: &ureq::Agent, path: &str, body: Value) -> Result<Value> {
        let mut req = agent.post(format!("{}{}", self.base, path));
        if let Some(t) = &self.token {
            req = req.header("x-terrarium-token", t);
        }
        let mut res = req
            .send_json(body)
            .map_err(|e| connect_error(e, &self.base))?;
        let status = res.status().as_u16();
        let body: Value = res.body_mut().read_json().unwrap_or(Value::Null);
        if status >= 400 {
            return Err(anyhow!(
                "app returned {status}: {}",
                body.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown error")
            ));
        }
        Ok(body)
    }

    pub fn is_up(&self) -> bool {
        self.get("/health")
            .map(|v| v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false))
            .unwrap_or(false)
    }
}

fn connect_error(e: ureq::Error, base: &str) -> anyhow::Error {
    anyhow!("app not reachable at {base} ({e})")
}
