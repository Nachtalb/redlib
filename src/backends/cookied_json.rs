//! Reddit's web host with the logged-in sessions from `REDLIB_REDDIT_COOKIE`.

use super::{check_json, Backend, Failure, Fetch, Kind};
use crate::client::{now_secs, reroll_client, session_cookies, IntoHyperResponse, ALTERNATIVE_REDDIT_URL_BASE, CLIENT};
use bytes::Buf;
use log::{trace, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
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
/// emulation, so switch to another one and try once more with the sessions
/// that still have budget.
async fn fetch(path: &str) -> Result<Value, Failure> {
	let path = &*Kind::json_path(path);
	let mut response = web_session_get(path).await;
	if let Err(refused) = response {
		if worth_a_new_browser(path, refused) && reroll_client("Reddit refused every session") {
			response = web_session_get(path).await;
		}
	}
	let response = response.map_err(no_answer)?;

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

/// Whether a request no session answered is worth retrying with another
/// browser: only when one was refused, not when they were all out of budget.
fn worth_a_new_browser(path: &str, refused: bool) -> bool {
	refused && blames_fingerprint(path)
}

/// What a request no session answered fails with.
fn no_answer(refused: bool) -> Failure {
	if refused {
		Failure::Blocked
	} else {
		Failure::RateLimited("Reddit rate limit exceeded".into())
	}
}

/// Tries `path` on Reddit's web host with each session that has budget, in
/// turn from a rotating index, and returns the first successful response, or
/// whether any session was refused for another reason than its budget.
async fn web_session_get(path: &str) -> Result<WreqResponse, bool> {
	let sessions = awake(session_cookies(), now_secs());
	let start = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
	let mut refused = false;
	for i in 0..sessions.len() {
		let (label, cookie) = &sessions[(start + i) % sessions.len()];
		let response = reddit_web_get(path, cookie).await;
		if let Ok(web_response) = &response {
			note_budget(label, cookie, web_response);
		}
		match response {
			Ok(web_response) if web_response.status().is_success() => {
				trace!("Retried {path} with session {label}: status {}", web_response.status());
				return Ok(web_response);
			}
			Ok(web_response) => {
				refused |= web_response.status() != wreq::StatusCode::TOO_MANY_REQUESTS;
				warn!("Session {label} was refused for {path}: status {}", web_response.status());
			}
			Err(e) => {
				refused = true;
				warn!("Couldn't reach Reddit's web host for {path}: {e}");
			}
		}
	}
	Err(refused)
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

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Host {
	Web,
	Api,
}

#[allow(dead_code)]
fn hosts(_path: &str) -> [Host; 2] {
	[Host::Web, Host::Api]
}

#[allow(dead_code)]
fn cached_token(_cookie: &str, _now: u64) -> Option<String> {
	None
}

#[allow(dead_code)]
fn keep_token(_cookie: &str, _token: String, _expires_in: u64, _now: u64) {}

#[allow(dead_code)]
fn drop_token(_cookie: &str) {}

/// `None` is a failed mint, kept until it may be retried.
#[allow(dead_code)]
static TOKENS: LazyLock<Mutex<HashMap<String, Minted>>> = LazyLock::new(Default::default);

type Minted = (Option<String>, u64);

#[allow(dead_code)]
const MINT_RETRY_SECS: u64 = 300;

#[allow(dead_code)]
fn mint_failed_recently(_cookie: &str, _now: u64) -> bool {
	false
}

#[allow(dead_code)]
fn mint_failed(_cookie: &str, _now: u64) {}

#[allow(dead_code)]
fn token_refused(_path: &str, _status: u16) -> bool {
	false
}

#[allow(dead_code)]
fn is_refusal(_host: Host, _status: u16) -> bool {
	true
}

#[allow(dead_code)]
async fn try_sessions<G, F>(_path: &str, _sessions: &[(String, String)], _start: usize, _get: G) -> Result<WreqResponse, bool>
where
	G: Fn(Host, String) -> F,
	F: Future<Output = Result<WreqResponse, String>>,
{
	Err(true)
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
	fn test_sessions_out_of_budget_are_a_rate_limit_not_a_block() {
		assert!(plain_rate_limit(&no_answer(false)));
		assert!(matches!(no_answer(true), Failure::Blocked));
	}

	#[test]
	fn test_only_refusals_switch_the_browser() {
		let post = "/r/rust/comments/abc123.json?raw_json=1";
		assert!(worth_a_new_browser(post, true));
		// a rate limit is the session's budget, not the browser
		assert!(!worth_a_new_browser(post, false));
		assert!(!worth_a_new_browser("/r/rust/hot.json?raw_json=1", true));
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

	#[test]
	fn test_listings_and_search_try_the_token_first() {
		for path in [
			"/r/rust/hot.json?raw_json=1",
			"/r/rust.json?raw_json=1",
			"/r/rust/top.json?t=week&raw_json=1",
			"/r/popular/hot.json?geo_filter=GLOBAL&raw_json=1",
			"/r/all+/hot.json?raw_json=1",
			"/search.json?q=rust&raw_json=1",
			"/r/rust/search.json?q=async&restrict_sr=on&raw_json=1",
			"/r/rust/api/widgets?raw_json=1",
		] {
			assert_eq!(hosts(path), [Host::Api, Host::Web], "{path}");
		}
	}

	#[test]
	fn test_posts_new_and_the_rest_try_the_cookie_first() {
		for path in [
			"/r/redditdev/comments/1wubcvf.json?raw_json=1",
			"/by_id/t3_1wubcvf.json?raw_json=1",
			"/r/rust/new.json?raw_json=1",
			"/r/all+/new.json?raw_json=1",
			"/r/rust/about.json?raw_json=1",
			"/user/spez/about.json?raw_json=1",
			"/r/rust/wiki/index.json?raw_json=1",
			"/duplicates/1wubcvf.json?raw_json=1",
		] {
			assert_eq!(hosts(path), [Host::Web, Host::Api], "{path}");
		}
	}

	#[test]
	fn test_token_is_reused_until_a_minute_before_it_expires() {
		keep_token("test-reuse", "token".into(), 3600, 1000);
		assert_eq!(cached_token("test-reuse", 1000).as_deref(), Some("token"));
		assert_eq!(cached_token("test-reuse", 1000 + 3600 - 61).as_deref(), Some("token"));
		assert_eq!(cached_token("test-reuse", 1000 + 3600 - 60), None);
		assert_eq!(cached_token("test-reuse-other", 1000), None);
	}

	#[test]
	fn test_failed_mint_waits_before_minting_again() {
		mint_failed("test-mint", 1000);
		assert!(mint_failed_recently("test-mint", 1000 + MINT_RETRY_SECS - 1));
		assert!(!mint_failed_recently("test-mint", 1000 + MINT_RETRY_SECS));
		assert_eq!(cached_token("test-mint", 1000), None);
		keep_token("test-mint-ok", "token".into(), 3600, 1000);
		assert!(!mint_failed_recently("test-mint-ok", 1000));
	}

	#[test]
	fn test_finished_tokens_are_forgotten() {
		keep_token("test-old", "token".into(), 10, 0);
		mint_failed("test-old-failed", 0);
		keep_token("test-fresh", "token".into(), 3600, 1000);
		let tokens = TOKENS.lock().unwrap_or_else(|e| e.into_inner()).clone();
		assert!(!tokens.contains_key("test-old") && !tokens.contains_key("test-old-failed"));
		assert!(tokens.contains_key("test-fresh"));
	}

	#[test]
	fn test_a_token_refused_where_it_should_work_is_distrusted() {
		assert!(token_refused("/r/rust/hot.json?raw_json=1", 403));
		assert!(token_refused("/search.json?q=rust&raw_json=1", 403));
		assert!(!token_refused("/r/rust/comments/abc.json?raw_json=1", 403));
		assert!(!token_refused("/r/rust/new.json?raw_json=1", 403));
		assert!(!token_refused("/r/rust/hot.json?raw_json=1", 429));
	}

	#[test]
	fn test_an_expired_token_is_no_refusal() {
		assert!(!is_refusal(Host::Api, 401));
		assert!(!is_refusal(Host::Api, 429));
		assert!(!is_refusal(Host::Web, 429));
		assert!(is_refusal(Host::Api, 403));
		assert!(is_refusal(Host::Web, 403));
		assert!(is_refusal(Host::Web, 401));
	}

	fn answer(status: u16, headers: &[(&str, &str)]) -> Result<WreqResponse, String> {
		let mut response = hyper::Response::builder().status(status);
		for (key, value) in headers {
			response = response.header(*key, *value);
		}
		Ok(WreqResponse::from(response.body("{}").unwrap()))
	}

	fn ask(path: &str, names: &[&str], reply: fn(Host, &str) -> Result<WreqResponse, String>) -> (Result<u16, bool>, Vec<(Host, String)>) {
		let calls = Mutex::new(Vec::new());
		let sessions: Vec<_> = names.iter().map(|name| session(name)).collect();
		let result = crate::client::block_on_shared(try_sessions(path, &sessions, 0, |host, cookie| {
			let name = cookie.trim_start_matches("reddit_session=").to_string();
			calls.lock().unwrap().push((host, name.clone()));
			async move { reply(host, &name) }
		}));
		(result.map(|r| r.status().as_u16()), calls.into_inner().unwrap())
	}

	#[test]
	fn test_refused_host_falls_back_to_the_other() {
		let (result, calls) = ask("/r/rust/hot.json?raw_json=1", &["loop-a"], |host, _| answer(if host == Host::Api { 403 } else { 200 }, &[]));
		assert_eq!(result, Ok(200));
		assert_eq!(calls, [(Host::Api, "loop-a".into()), (Host::Web, "loop-a".into())]);
	}

	#[test]
	#[sealed_test]
	fn test_session_out_of_budget_skips_its_other_host() {
		let (result, calls) = ask("/r/rust/hot.json?raw_json=1", &["loop-tired", "loop-b"], |_, name| match name {
			"loop-tired" => answer(429, &[("x-ratelimit-reset", "100")]),
			_ => answer(200, &[]),
		});
		assert_eq!(result, Ok(200));
		assert_eq!(calls, [(Host::Api, "loop-tired".into()), (Host::Api, "loop-b".into())]);
	}

	#[test]
	#[sealed_test]
	fn test_expired_tokens_alone_are_no_refusal() {
		let (result, _) = ask("/r/rust/hot.json?raw_json=1", &["loop-401"], |host, _| {
			answer(if host == Host::Api { 401 } else { 429 }, &[])
		});
		assert_eq!(result, Err(false));
	}

	#[test]
	fn test_session_without_a_token_skips_the_api() {
		// `mint_failed` would prune other tests' entries
		TOKENS
			.lock()
			.unwrap_or_else(|e| e.into_inner())
			.insert("reddit_session=loop-notoken".into(), (None, u64::MAX));
		let (_, calls) = ask("/r/rust/hot.json?raw_json=1", &["loop-notoken"], |_, _| answer(200, &[]));
		assert_eq!(calls, [(Host::Web, "loop-notoken".into())]);
	}

	#[test]
	fn test_dropped_token_is_minted_again() {
		keep_token("test-drop", "token".into(), 3600, 1000);
		drop_token("test-drop");
		assert_eq!(cached_token("test-drop", 1000), None);
	}
}
