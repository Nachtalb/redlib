//! Reddit's web host with the logged-in sessions from `REDLIB_REDDIT_COOKIE`.

use super::{check_json, Backend, Failure, Fetch, Kind};
use crate::client::{now_secs, reroll_client, session_cookies, IntoHyperResponse, ALTERNATIVE_REDDIT_URL_BASE, CLIENT};
use bytes::Buf;
use log::{trace, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use wreq::{header as wreq_header, Response as WreqResponse};

pub struct CookiedJson;

impl Backend for CookiedJson {
	fn name(&self) -> &'static str {
		"cookied-json"
	}

	fn available(&self) -> bool {
		!session_cookies().is_empty()
	}

	fn logged_in(&self) -> bool {
		true
	}

	fn cost(&self) -> f64 {
		2.0
	}

	fn fetch<'a>(&'a self, path: &'a str, _quarantine: bool) -> Fetch<'a> {
		Box::pin(fetch(path))
	}
}

/// Index of the session the next request starts with.
static NEXT_SESSION: AtomicUsize = AtomicUsize::new(0);

/// Sessions take turns; a refused one passes the request on to the next.
/// When every session is refused, Reddit may have flagged the browser
/// emulation, so switch to another one and try once more.
async fn fetch(path: &str) -> Result<Value, Failure> {
	let path = &*Kind::json_path(path);
	let sessions = awake(session_cookies(), now_secs());
	if sessions.is_empty() {
		return Err(Failure::RateLimited("Reddit rate limit exceeded".into()));
	}
	let mut response = web_session_get(path, &sessions).await;
	if response.is_none() && blames_fingerprint(path) && reroll_client("Reddit refused every session") {
		response = web_session_get(path, &sessions).await;
	}
	let response = response.ok_or(Failure::Blocked)?;

	let body = response
		.into_hyper_response()
		.into_body()
		.collect_bytes()
		.await
		.map_err(|e| Failure::Network(format!("Failed receiving body from Reddit: {e} | {path}")))?;
	// An empty or non-JSON body from the web host is a refusal, not the
	// OAuth rate limit.
	match serde_json::from_reader(body.reader()) {
		Ok(json) => check_json(json, path).await,
		Err(_) => Err(Failure::Blocked),
	}
}

/// Subreddit listings and search are refused to sessions on every fingerprint
/// (probe, November 2026), so switching away from a working one can't help.
fn blames_fingerprint(path: &str) -> bool {
	!matches!(Kind::of(path), Kind::Listing | Kind::Search)
}

/// Tries `path` on Reddit's web host with each session in turn, starting at a
/// rotating index, and returns the first successful response.
async fn web_session_get(path: &str, sessions: &[(String, String)]) -> Option<WreqResponse> {
	let start = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
	for i in 0..sessions.len() {
		let (label, cookie) = &sessions[(start + i) % sessions.len()];
		let response = reddit_web_get(path, cookie).await;
		if let Ok(web_response) = &response {
			note_budget(label, cookie, web_response);
		}
		match response {
			Ok(web_response) if web_response.status().is_success() => {
				trace!("Retried {path} with session {label}: status {}", web_response.status());
				return Some(web_response);
			}
			Ok(web_response) => warn!("Session {label} was refused for {path}: status {}", web_response.status()),
			Err(e) => warn!("Couldn't reach Reddit's web host for {path}: {e}"),
		}
	}
	None
}

/// How long a session rests after a response with `status` and Reddit's
/// `x-ratelimit-remaining` / `x-ratelimit-reset` headers: until the window
/// resets once it's used up, so its next requests go to the other sessions
/// instead of being refused one by one.
fn rest_secs(status: u16, remaining: Option<f64>, reset: Option<f64>) -> Option<u64> {
	let used_up = status == 429 || (status < 400 && remaining.is_some_and(|left| left < 1.0));
	used_up.then(|| reset.unwrap_or(60.0).clamp(1.0, 600.0) as u64)
}

/// Sessions resting after using up their budget, with when they may go again.
static RESTING: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

/// `sessions` without the ones resting at `now`.
fn awake(sessions: Vec<(String, String)>, now: u64) -> Vec<(String, String)> {
	let resting = RESTING.lock().unwrap_or_else(|e| e.into_inner());
	sessions.into_iter().filter(|(_, cookie)| resting.get(cookie).is_none_or(|until| *until <= now)).collect()
}

/// Sends `cookie` to rest when `response` says its budget is used up.
fn note_budget(label: &str, cookie: &str, response: &WreqResponse) {
	let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()?.parse::<f64>().ok());
	if let Some(secs) = rest_secs(response.status().as_u16(), header("x-ratelimit-remaining"), header("x-ratelimit-reset")) {
		warn!("Session {label} used up its request budget, resting it for {secs}s");
		let now = now_secs();
		send_to_rest(cookie, now + secs, now);
	}
}

/// Rests `cookie` until `until`, forgetting the rests that are over at `now`.
fn send_to_rest(cookie: &str, until: u64, now: u64) {
	let mut resting = RESTING.lock().unwrap_or_else(|e| e.into_inner());
	resting.retain(|_, until| *until > now);
	resting.insert(cookie.to_string(), until);
}

/// Fetches `path` from Reddit's web host as the logged-in session `cookie`.
///
/// Only the cookie is overridden: the client's browser emulation already sends
/// the TLS, HTTP/2 and default headers Reddit expects next to a session cookie,
/// and a mismatch between them is what gets a cookie refused. Redirects are not
/// followed, because Reddit sends an expired session to its login page and that
/// must be reported rather than parsed as JSON.
async fn reddit_web_get(path: &str, cookie: &str) -> Result<WreqResponse, String> {
	let url = format!("{ALTERNATIVE_REDDIT_URL_BASE}{path}");

	CLIENT.load_full().get(&url).header(wreq_header::COOKIE, cookie).send().await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;
	use sealed_test::prelude::*;

	#[test]
	fn test_listing_and_search_refusals_keep_the_fingerprint() {
		assert!(!blames_fingerprint("/r/rust/hot.json?raw_json=1"));
		assert!(!blames_fingerprint("/search.json?q=rust&raw_json=1"));
		assert!(blames_fingerprint("/r/rust/comments/abc123.json?raw_json=1"));
		assert!(blames_fingerprint("/user/spez/about.json?raw_json=1"));
	}

	fn session(name: &str) -> (String, String) {
		(format!("#{name}"), format!("reddit_session={name}"))
	}

	fn rest(name: &str, until: u64) {
		RESTING.lock().unwrap_or_else(|e| e.into_inner()).insert(session(name).1, until);
	}

	#[test]
	fn test_resting_sessions_are_skipped_until_their_rest_ends() {
		rest("tired", 100);
		let sessions = vec![session("tired"), session("fresh")];
		assert_eq!(awake(sessions.clone(), 99), [session("fresh")]);
		assert_eq!(awake(sessions.clone(), 100), sessions);
	}

	/// A rate limit that doesn't tell visitors the instance has sessions.
	fn plain_rate_limit(failure: &Failure) -> bool {
		matches!(failure, Failure::RateLimited(msg) if !msg.to_lowercase().contains("session"))
	}

	#[test]
	#[sealed_test(env = [("REDLIB_REDDIT_COOKIE", "reddit_session=a|reddit_session=b")])]
	fn test_every_session_resting_is_a_rate_limit_without_a_request() {
		rest("a", u64::MAX);
		rest("b", u64::MAX);
		let result = crate::client::block_on_shared(fetch("/r/rust/hot.json?raw_json=1"));
		assert!(result.as_ref().is_err_and(plain_rate_limit), "{result:?}");
	}

	#[test]
	fn test_finished_rests_are_forgotten() {
		// sessions removed from the session file mustn't stay in memory
		send_to_rest("reddit_session=gone", 10, 0);
		send_to_rest("reddit_session=new", 100, 20);
		let resting = RESTING.lock().unwrap_or_else(|e| e.into_inner()).clone();
		assert!(!resting.contains_key("reddit_session=gone"));
		assert_eq!(resting.get("reddit_session=new"), Some(&100));
	}

	#[test]
	fn test_used_up_session_rests_until_the_window_resets() {
		assert_eq!(rest_secs(200, Some(0.0), Some(120.0)), Some(120));
		assert_eq!(rest_secs(429, None, Some(30.0)), Some(30));
		// no reset header: a minute
		assert_eq!(rest_secs(429, None, None), Some(60));
		// never longer than Reddit's ten minute window
		assert_eq!(rest_secs(429, Some(0.0), Some(5000.0)), Some(600));
	}

	#[test]
	fn test_session_with_requests_left_keeps_working() {
		assert_eq!(rest_secs(200, Some(42.0), Some(120.0)), None);
		assert_eq!(rest_secs(200, None, None), None);
		assert_eq!(rest_secs(403, Some(10.0), Some(120.0)), None);
	}
}
