//! The TalkBank archive HTTP client.
//!
//! Every route is a `POST` with a JSON body against `https://sla2.talkbank.org`.
//! Authentication is by cookie (`talkbank`, domain `.talkbank.org`, 24 hours),
//! so a single session covers both the JSON calls and the downloads, which live
//! on `talkbank.org`.
//!
//! Things learned against the live service, worth keeping:
//!  * the password field in the login body is **`pswd`**, not `password`;
//!  * `getTranscriptSummary` and `getParticipantSummary` answer
//!    `{authStatus, colHeadings, data}` **with no `respMsg`**: a generic helper
//!    that unwraps `respMsg` would break on two routes out of three;
//!  * rows in `data` must never be indexed by position, only resolved by name
//!    through `colHeadings`;
//!  * five routes documented by the official clients (`getNgrams`,
//!    `getTokenSummary`, `getUtteranceSummary`, `getStats`, `cql`) answer 404
//!    today. A route that disappears **degrades**, it is not an error.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

pub const BASE: &str = "https://sla2.talkbank.org";
pub const DATA_BASE: &str = "https://talkbank.org/data";

/// The outcome of a call, classified once here so that every caller can react
/// consistently.
#[derive(Debug)]
pub enum ApiError {
    /// Sign-in required.
    AuthRequired,
    /// The route is gone (404/405): the caller should degrade quietly.
    RouteGone(String),
    /// Server error (5xx).
    Server(u16),
    /// A 200 response that cannot be interpreted.
    Malformed(String),
    /// No network, DNS, TLS, timeout.
    Network(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::AuthRequired => write!(f, "sign-in required"),
            ApiError::RouteGone(r) => write!(f, "the route \"{r}\" no longer exists"),
            ApiError::Server(c) => write!(f, "server error ({c})"),
            ApiError::Malformed(m) => write!(f, "unexpected response: {m}"),
            ApiError::Network(m) => write!(f, "network unreachable: {m}"),
        }
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// True when the caller should simply not show that piece of data.
    pub fn is_degradable(&self) -> bool {
        matches!(self, ApiError::RouteGone(_))
    }
}

/// The outcome of a sign-in attempt, with the cases the server really separates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    /// Named explicitly instead of `Ok`: importing the enum would shadow
    /// `Result`'s own `Ok` and calling code would stop compiling.
    Success,
    /// Email or password do not match (`NOT_MATCHED`).
    WrongCredentials,
    /// Account created but email not confirmed (`EMAIL_NOT_VERIFIED`).
    EmailNotVerified,
    /// A code we do not know: show it as-is rather than swallowing it.
    Other(String),
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    cookies: Arc<reqwest::cookie::Jar>,
    /// Downloadability answers already obtained, for this session.
    probes: Arc<std::sync::RwLock<std::collections::HashMap<Vec<String>, Downloadable>>>,
}

impl Client {
    pub fn new() -> Result<Client, ApiError> {
        let cookies = Arc::new(reqwest::cookie::Jar::default());
        let http = reqwest::Client::builder()
            .cookie_provider(cookies.clone())
            .user_agent(concat!("talkbank-desktop/", env!("CARGO_PKG_VERSION")))
            // Latency is unpredictable: getParticipantSummary took 1.3 s on one
            // corpus and 35.8 s on another. A tight timeout would turn a slow
            // answer into an error.
            .timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;
        Ok(Client {
            http,
            cookies,
            probes: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// The single choke point: every JSON route goes through here, so outcome
    /// classification is written once.
    pub async fn post(&self, route: &str, body: Value) -> Result<Value, ApiError> {
        let url = format!("{BASE}/{route}");
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND
            || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            return Err(ApiError::RouteGone(route.to_string()));
        }
        if status.is_server_error() {
            return Err(ApiError::Server(status.as_u16()));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let value: Value = serde_json::from_str(&text).map_err(|_| {
            ApiError::Malformed(text.chars().take(160).collect::<String>())
        })?;

        // `respMsg: "auth"` is how getCHAT says "you are not signed in".
        if value.get("respMsg").and_then(|v| v.as_str()) == Some("auth") {
            return Err(ApiError::AuthRequired);
        }
        Ok(value)
    }

    // ------------------------------------------------------------- sign-in

    pub async fn login(&self, email: &str, password: &str) -> Result<LoginOutcome, ApiError> {
        // The field is `pswd`: with `password` the server answers NOT_MATCHED
        // even for correct credentials.
        let v = self
            .post("logInUser", json!({"email": email, "pswd": password}))
            .await?;

        if v.get("success").and_then(Value::as_bool) == Some(true) {
            // While signed out every probe answered "sign-in required": the
            // answers cached before the login no longer hold.
            self.forget_probes();
            return Ok(LoginOutcome::Success);
        }
        Ok(match v.get("respMsg").and_then(Value::as_str) {
            Some("NOT_MATCHED") => LoginOutcome::WrongCredentials,
            Some("EMAIL_NOT_VERIFIED") => LoginOutcome::EmailNotVerified,
            Some(other) => LoginOutcome::Other(other.to_string()),
            None => LoginOutcome::Other(String::new()),
        })
    }

    pub async fn is_logged_in(&self) -> Result<bool, ApiError> {
        let v = self.post("isLoggedIn", json!({})).await?;
        Ok(v.get("respMsg").and_then(Value::as_bool).unwrap_or(false)
            || v.pointer("/authStatus/loggedIn")
                .and_then(Value::as_bool)
                .unwrap_or(false))
    }

    pub async fn logout(&self) -> Result<(), ApiError> {
        self.forget_probes();
        self.post("logOutUser", json!({})).await.map(|_| ())
    }

    /// Forgets the probe answers. Call it whenever the session changes, because
    /// the session is what determines them.
    pub fn forget_probes(&self) {
        if let Ok(mut c) = self.probes.write() {
            c.clear();
        }
    }

    /// Whether the session is authorised for an archive path.
    pub async fn has_access(&self, path: &str) -> Result<bool, ApiError> {
        let v = self
            .post("sessionHasAuth", json!({"rootName": "data", "path": path}))
            .await?;
        Ok(v.get("auth").and_then(Value::as_bool).unwrap_or(false))
    }

    // ------------------------------------------------------------- catalogue

    /// The complete archive tree: 4.3 MB, public.
    pub async fn tree(&self) -> Result<Value, ApiError> {
        self.post("getAnnoPathTrees", json!({})).await
    }

    // -------------------------------------------------------------- metadata

    /// Summary of a folder's transcripts. Public.
    ///
    /// The path can be any length: verified that the server answers both
    /// `["ca","ATC"]` and `["phon","Eng-NA","Davis"]`.
    pub async fn transcript_summary(&self, path: &[String]) -> Result<Table, ApiError> {
        let v = self.post("getTranscriptSummary", query_for(path)).await?;
        Table::from_response(&v)
    }

    /// Summary of the participants. Public, but it can be slow: 35.8 seconds
    /// measured on a large corpus.
    pub async fn participant_summary(&self, path: &[String]) -> Result<Table, ApiError> {
        let v = self.post("getParticipantSummary", query_for(path)).await?;
        Table::from_response(&v)
    }

    /// Whether a folder is downloadable.
    ///
    /// This cannot be deduced from the tree: the corpus level differs from bank
    /// to bank, and `childes/Eng-NA/Brown` downloads while `childes/Eng-NA` does
    /// not, even though both hold only subfolders. A HEAD request settles it
    /// without transferring anything.
    ///
    /// **Only with an open session.** Without one the access gate answers `200`
    /// with `text/html` for any path and the probe would say "yes" to
    /// everything: that is why the HTTP status alone is not enough and the
    /// content type has to be inspected too.
    pub async fn is_downloadable(&self, path: &[String]) -> Downloadable {
        if let Some(known) = self.cached_downloadable(path) {
            return known;
        }
        let url = corpus_zip_url(path);
        let resp = match self.http.head(&url).send().await {
            Ok(r) => r,
            Err(e) => return Downloadable::Unknown(e.to_string()),
        };
        let status = resp.status();
        // 401: the account is fine, but this bank needs separate permission
        // (aphasia, samtale, psychosis…).
        let outcome = if status == reqwest::StatusCode::UNAUTHORIZED {
            Downloadable::NeedsPermission
        } else if status == reqwest::StatusCode::NOT_FOUND {
            // **Only** a 404 means "not a corpus": measured, it arrives as
            // `text/plain` and nine bytes. Treating a 503 or a 429 as "no" would
            // be worse than useless — it would drop a corpus from the plan in
            // silence, and would descend into its children, multiplying requests
            // exactly while the server is struggling.
            Downloadable::No
        } else if !status.is_success() {
            Downloadable::Unknown(format!("HTTP {}", status.as_u16()))
        } else {
            let ct = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if ct.starts_with("text/html") {
                Downloadable::SignInRequired
            } else {
                Downloadable::Yes
            }
        };

        // In memory, not on disk: the archive gets reorganised (two paths that
        // used to be corpora answer 404 today), and a plan built on stale
        // answers would queue folders that no longer exist.
        if matches!(outcome, Downloadable::Yes | Downloadable::No | Downloadable::NeedsPermission) {
            if let Ok(mut c) = self.probes.write() {
                c.insert(path.to_vec(), outcome.clone());
            }
        }
        outcome
    }

    /// The answer already known for a path, if we asked in this session. Saves
    /// asking the same thing twice seconds apart — which happens constantly,
    /// because the corpus page and the branch planner probe the same path.
    pub fn cached_downloadable(&self, path: &[String]) -> Option<Downloadable> {
        self.probes.read().ok()?.get(path).cloned()
    }

    pub fn cookie_jar(&self) -> Arc<reqwest::cookie::Jar> {
        self.cookies.clone()
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

/// The result of the downloadability probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Downloadable {
    Yes,
    /// Not a corpus folder: either a collection, or a subfolder inside a corpus.
    No,
    /// Undecidable without a session: the access gate answers identically for a
    /// corpus and for a collection.
    SignInRequired,
    /// This bank needs specific permission.
    NeedsPermission,
    /// Could not be established (network).
    Unknown(String),
}

/// The body of a metadata query.
///
/// The path starts with the bank and continues with the folders: the doubled
/// form TBDBpy uses does not work on this route.
fn query_for(path: &[String]) -> Value {
    let bank = path.first().cloned().unwrap_or_default();
    json!({"queryVals": {
        "corpusName": bank,
        "corpora": [path],
        "lang": [], "media": [], "age": [], "gender": [],
        "designType": [], "activityType": [], "groupType": [],
        "respType": "JSON", "maxRows": false
    }})
}

/// A `{colHeadings, data}` table, addressed by column name.
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub headings: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    pub fn from_response(v: &Value) -> Result<Table, ApiError> {
        let headings: Vec<String> = v
            .get("colHeadings")
            .and_then(Value::as_array)
            .ok_or_else(|| ApiError::Malformed("colHeadings missing".into()))?
            .iter()
            .map(|h| h.as_str().unwrap_or_default().to_string())
            .collect();
        let rows = v
            .get("data")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r.as_array().cloned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(Table { headings, rows })
    }

    pub fn column(&self, name: &str) -> Option<usize> {
        self.headings.iter().position(|h| h == name)
    }

    /// Text value of a cell. Also returns `None` for the literal string
    /// `"NULL"`, which is how the server signals absence.
    pub fn get<'a>(&'a self, row: &'a [Value], name: &str) -> Option<&'a str> {
        let idx = self.column(name)?;
        match row.get(idx) {
            Some(Value::String(s)) if s != "NULL" && !s.is_empty() => Some(s),
            _ => None,
        }
    }

    /// Distinct values of a column, in order of first appearance.
    pub fn distinct(&self, name: &str) -> Vec<String> {
        let mut seen = Vec::new();
        for row in &self.rows {
            if let Some(v) = self.get(row, name) {
                if !seen.iter().any(|s: &String| s == v) {
                    seen.push(v.to_string());
                }
            }
        }
        seen
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// URL of the zip for a corpus folder.
pub fn corpus_zip_url(path: &[String]) -> String {
    format!("{DATA_BASE}/{}?f=zip", path.join("/"))
}

/// Public description page, to open in a browser: it holds the required citation
/// and the history of the corpus.
pub fn corpus_page_url(path: &[String]) -> String {
    let bank = path.first().map(String::as_str).unwrap_or("childes");
    let rest = path[1..].join("/");
    format!("https://{bank}.talkbank.org/access/{rest}.html")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_table_is_read_by_name_not_by_position() {
        // columns in a different order than expected: it must work regardless
        let v = json!({
            "authStatus": {"loggedIn": false},
            "colHeadings": ["media", "path", "filename", "extra"],
            "data": [["audio", "childes/Eng-NA/Brown/adam01", "adam01", 7]]
        });
        let t = Table::from_response(&v).unwrap();
        let row = &t.rows[0];
        assert_eq!(t.get(row, "filename"), Some("adam01"));
        assert_eq!(t.get(row, "media"), Some("audio"));
        assert_eq!(t.get(row, "no-such-column"), None);
    }

    #[test]
    fn the_servers_null_string_counts_as_absent() {
        let v = json!({"colHeadings": ["a", "b"], "data": [["NULL", ""]]});
        let t = Table::from_response(&v).unwrap();
        assert_eq!(t.get(&t.rows[0], "a"), None);
        assert_eq!(t.get(&t.rows[0], "b"), None);
    }

    #[test]
    fn missing_or_extra_columns_do_not_fail() {
        let v = json!({"colHeadings": ["path"], "data": [["x"], ["y", "extra"]]});
        let t = Table::from_response(&v).unwrap();
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.get(&t.rows[1], "path"), Some("y"));
    }

    #[test]
    fn without_colheadings_the_response_is_malformed() {
        let e = Table::from_response(&json!({"respMsg": "dunno"})).unwrap_err();
        assert!(matches!(e, ApiError::Malformed(_)));
    }

    #[test]
    fn distinct_values_in_order_of_appearance() {
        let v = json!({"colHeadings": ["lang"],
                       "data": [["eng"], ["spa"], ["eng"], ["NULL"]]});
        let t = Table::from_response(&v).unwrap();
        assert_eq!(t.distinct("lang"), ["eng", "spa"]);
    }

    fn p(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_urls_have_the_shape_verified_against_the_site() {
        assert_eq!(
            corpus_zip_url(&p(&["childes", "Eng-NA", "Brown"])),
            "https://talkbank.org/data/childes/Eng-NA/Brown?f=zip"
        );
        // bank with no collection level: only two elements
        assert_eq!(
            corpus_zip_url(&p(&["ca", "ATC"])),
            "https://talkbank.org/data/ca/ATC?f=zip"
        );
        assert_eq!(
            corpus_page_url(&p(&["childes", "Eng-NA", "Brown"])),
            "https://childes.talkbank.org/access/Eng-NA/Brown.html"
        );
    }

    #[test]
    fn the_query_body_carries_the_whole_path() {
        let q = query_for(&p(&["childes", "Eng-NA", "Brown"]));
        assert_eq!(
            q["queryVals"]["corpora"],
            json!([["childes", "Eng-NA", "Brown"]]),
            "TBDBpy's doubled form does not work on this route"
        );
        assert_eq!(q["queryVals"]["corpusName"], "childes");
        // and it holds for the short paths of banks with no collection level
        let q = query_for(&p(&["ca", "ATC"]));
        assert_eq!(q["queryVals"]["corpora"], json!([["ca", "ATC"]]));
        assert_eq!(q["queryVals"]["respType"], "JSON");
    }

    #[test]
    fn a_vanished_route_is_degradable_a_network_error_is_not() {
        assert!(ApiError::RouteGone("getNgrams".into()).is_degradable());
        assert!(!ApiError::Network("dns".into()).is_degradable());
        assert!(!ApiError::AuthRequired.is_degradable());
    }
}
