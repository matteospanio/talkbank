//! The Batchalign3 control-plane client.
//!
//! On progress: `GET /jobs/{id}/stream` exists as SSE, but their `openapi.json`
//! declares it without an event schema — we would be relying on an undocumented
//! format that can change without warning. `GET /jobs/{id}` is typed instead and
//! already carries `completed_files`, `total_files` and `current_file`: polling
//! it is enough. A Batchalign job runs for minutes, so one-second granularity is
//! more than sufficient.

use std::time::Duration;

use crate::types::{Health, JobInfo, Status, Submission};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Batchalign is unreachable: {0}")]
    Unreachable(String),
    #[error("request rejected: {0}")]
    Rejected(String),
    #[error("job not found")]
    NotFound,
    #[error("unexpected response: {0}")]
    Malformed(String),
}

#[derive(Clone)]
pub struct Client {
    base: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base: impl Into<String>) -> Client {
        Client {
            base: base.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn at_port(port: u16) -> Client {
        Client::new(crate::server::base_url(port))
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let r = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::NotFound);
        }
        let text = r.text().await.map_err(|e| Error::Unreachable(e.to_string()))?;
        serde_json::from_str(&text)
            .map_err(|e| Error::Malformed(format!("{e}: {}", text.chars().take(120).collect::<String>())))
    }

    pub async fn health(&self) -> Result<Health, Error> {
        self.get("/health").await
    }

    pub async fn submit(&self, job: &Submission) -> Result<JobInfo, Error> {
        let r = self
            .http
            .post(format!("{}/jobs", self.base))
            .json(job)
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;

        let status = r.status();
        let text = r.text().await.map_err(|e| Error::Unreachable(e.to_string()))?;
        if status == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::Rejected(text.chars().take(300).collect()));
        }
        serde_json::from_str(&text)
            .map_err(|e| Error::Malformed(format!("{e}: {}", text.chars().take(120).collect::<String>())))
    }

    pub async fn job(&self, id: &str) -> Result<JobInfo, Error> {
        self.get(&format!("/jobs/{id}")).await
    }

    pub async fn cancel(&self, id: &str) -> Result<(), Error> {
        self.http
            .post(format!("{}/jobs/{id}/cancel", self.base))
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        Ok(())
    }

    /// Follows a job to the end, reporting every change.
    ///
    /// `on_update` returns `false` to cancel: in that case the job is stopped on
    /// the server too, not merely stopped being watched.
    pub async fn follow(
        &self,
        id: &str,
        mut on_update: impl FnMut(&JobInfo) -> bool,
    ) -> Result<JobInfo, Error> {
        loop {
            let info = self.job(id).await?;
            let keep_going = on_update(&info);
            if !keep_going {
                let _ = self.cancel(id).await;
                return self.job(id).await;
            }
            if info.status.is_final() {
                return Ok(info);
            }
            tokio::time::sleep(Duration::from_millis(900)).await;
        }
    }

    /// The files produced by a finished job.
    pub async fn results(&self, id: &str) -> Result<Vec<String>, Error> {
        let v: serde_json::Value = self.get(&format!("/jobs/{id}/results")).await?;
        // The exact shape is not in their schema: we accept both a list of
        // strings and objects with a name field, without breaking if it changes.
        Ok(match &v {
            serde_json::Value::Array(items) => items
                .iter()
                .filter_map(|i| {
                    i.as_str()
                        .map(str::to_string)
                        .or_else(|| i.get("filename").and_then(|f| f.as_str()).map(str::to_string))
                })
                .collect(),
            serde_json::Value::Object(o) => o
                .get("files")
                .and_then(|f| f.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| i.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        })
    }
}

/// A readable job status, for the interface.
pub fn describe(info: &JobInfo) -> String {
    match info.status {
        Status::Queued => "queued".into(),
        Status::Running => match (&info.current_file, info.total_files) {
            (Some(f), n) if n > 0 => format!("{} ({}/{})", f, info.completed_files + 1, n),
            (Some(f), _) => f.clone(),
            (None, _) => "running".into(),
        },
        Status::Completed => "completed".into(),
        Status::Cancelled => "cancelled".into(),
        Status::Failed | Status::WritebackFailed | Status::Interrupted | Status::Unknown => info
            .error
            .clone()
            .unwrap_or_else(|| format!("{:?}", info.status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(json: &str) -> JobInfo {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn the_running_status_says_which_file_we_are_on() {
        let j = job(
            r#"{"job_id":"a","status":"running","total_files":3,"completed_files":1,
                "current_file":"adam02.cha"}"#,
        );
        assert_eq!(describe(&j), "adam02.cha (2/3)");
    }

    #[test]
    fn an_error_shows_the_servers_message_not_the_status_name() {
        let j = job(r#"{"job_id":"a","status":"failed","error":"model not found"}"#);
        assert_eq!(describe(&j), "model not found");
    }

    #[test]
    fn with_no_message_it_falls_back_to_the_status() {
        let j = job(r#"{"job_id":"a","status":"failed"}"#);
        assert_eq!(describe(&j), "Failed");
    }

    #[test]
    fn a_client_only_ever_points_at_localhost() {
        let c = Client::at_port(18000);
        assert!(c.base.starts_with("http://127.0.0.1:"));
    }
}
