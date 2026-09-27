# Changes compared to upstream Redlib

This fork is based on [redlib-org/redlib](https://github.com/redlib-org/redlib) and tracks its `main` branch. It adds unmerged upstream pull requests, changes from [Sparronator9999/redlib](https://github.com/Sparronator9999/redlib), and its own work.

## Container images

| | Upstream | This fork |
|---|---|---|
| Registry | `quay.io/redlib/redlib` | `ghcr.io/nachtalb/redlib` (forks publish to `ghcr.io/<owner>/<repo>`) |
| Platforms | `linux/amd64`, `linux/arm64`, `linux/arm/v7` | `linux/amd64`, `linux/386`, `linux/arm64`, `linux/arm/v7`, `linux/arm/v6` |
| Base | Alpine | Distroless `static:nonroot`: static musl binary, no shell or package manager, runs as uid 65532, ~16 MB |
| Tags | `latest` | `latest` (from `main`) and `sha-<short commit>` |
| Health check | Docker `HEALTHCHECK` with `wget /settings` | `GET /healthz` for the orchestrator to probe (the image has no `wget`) |

All platforms are cross-compiled natively on the build host and published under one multi-arch tag, so `docker pull` picks the right one.

```bash
docker run -d --name redlib -p 8080:8080 ghcr.io/nachtalb/redlib:latest
```

No release binaries are published. Build from source with `cargo build --release`.

## Fixes

Bugs present in upstream Redlib that are fixed here:

- **Subscriptions and filters were wiped** when saving a single setting ([redlib-org#562](https://github.com/redlib-org/redlib/issues/562)).
- **Instance default post sort ignored** until the user saved their settings once ([redlib-org#521](https://github.com/redlib-org/redlib/pull/521)).
- **Crash on a deleted or invalid comment link**: out-of-bounds access when a thread has no comments ([redlib-org#542](https://github.com/redlib-org/redlib/pull/542)).
- **Share links to posts on a user's profile** (`/u/<name>/s/<id>`) showed "Nothing here" ([redlib-org#560](https://github.com/redlib-org/redlib/pull/560)).
- **Videos transcoded by Reddit** (`rich:video`, e.g. external GIF hosts) had no player; CMAF video format is supported ([redlib-org#549](https://github.com/redlib-org/redlib/pull/549)).
- **Inconsistent margins** inside posts and comments, and headings without margins; empty zero-width-space paragraphs no longer add gaps ([redlib-org#413](https://github.com/redlib-org/redlib/pull/413)).
- **Autoplaying videos played with sound**: with "Autoplay videos" on, every video on a listing could start with audio at once. Autoplay is now muted.
- **Pausing an HLS video while it loads** was ignored and the video started playing anyway.
- **Settings codes broke on every new setting**: codes exported before a new field was added failed to restore. See [Settings codes](#settings-codes).
- **`/info.txt` showed the wrong values** next to its labels (e.g. "Pushshift frontend" showed the RSS setting).
- **Comment search had no submit button.**
- **Large numbers** show as `10K` instead of `10.0K`.
- **Comment collapse arrow** has a bigger touch area on mobile ([redlib-org#520](https://github.com/redlib-org/redlib/pull/520)).
- **Search box focus outline** covers the whole search form, not just the input ([redlib-org#524](https://github.com/redlib-org/redlib/pull/524)).
- **Shutdown cut off running downloads**: on SIGTERM or Ctrl+C, the server now finishes in-flight requests before exiting.

## Features

From upstream pull requests that are not merged upstream:

- **RedGifs videos** play inline, proxied through the instance ([redlib-org#507](https://github.com/redlib-org/redlib/pull/507)). Can be turned off with `REDLIB_ENABLE_REDGIFS=off`.
- **GIPHY GIFs in comments** are proxied and embedded ([redlib-org#561](https://github.com/redlib-org/redlib/pull/561)).
- **Keyboard navigation** on post pages ([redlib-org#422](https://github.com/redlib-org/redlib/pull/422)): `j` / `k` next / previous comment, `t` / `p` next / previous thread, `Enter` open the post link, `Shift+Enter` open it in a new tab.
- **Collapse a comment** by clicking its indent line ([redlib-org#291](https://github.com/redlib-org/redlib/pull/291)).
- **Lazy-loaded post images** ([redlib-org#539](https://github.com/redlib-org/redlib/pull/539)).
- **Geo filter** as a user setting instead of a URL parameter ([redlib-org#566](https://github.com/redlib-org/redlib/pull/566)), with country flags and names in the dropdown.
- **Configurable source code link** in the footer ([redlib-org#568](https://github.com/redlib-org/redlib/pull/568)).
- **Better RSS media**: images, galleries and videos render in RSS readers, and content is HTML-escaped ([redlib-org#572](https://github.com/redlib-org/redlib/pull/572)).

From [Sparronator9999/redlib](https://github.com/Sparronator9999/redlib):

- **Clean URLs**: optional setting that strips tracking parameters (`utm_*`, `si=`, `fbclid`, …) from external links (upstream [redlib-org#460](https://github.com/redlib-org/redlib/pull/460)).
- **Posts per page** (1–100) setting.
- **Maximum comment thread depth** setting.
- **Return to the previous page** after saving settings.

Own features:

- **GIF posts play like GIFs**: muted, looping, no controls, independent of the autoplay setting.
- **One video at a time**: with autoplay on, only the video most in view plays. It pauses when scrolled away and the next one takes over.
- **`/healthz` endpoint**: returns `200 ok` without contacting Reddit.

## Configuration

### New instance settings

| Variable | Default | Description |
|---|---|---|
| `REDLIB_ENABLE_REDGIFS` | `on` | Proxy RedGifs videos and play them inline. `off` leaves RedGifs as plain links. |
| `REDLIB_SOURCE_URL` | `https://github.com/redlib-org/redlib` | Source code link in the footer. |

### New user settings

Each has an instance default via `REDLIB_DEFAULT_<NAME>`:

| Setting | Variable | Default |
|---|---|---|
| Geo filter | `REDLIB_DEFAULT_GEO_FILTER` | `GLOBAL` |
| Clean URLs | `REDLIB_DEFAULT_CLEAN_URLS` | `off` |
| Posts per page | `REDLIB_DEFAULT_POSTS_PER_PAGE` | `25` |
| Max comment thread depth | `REDLIB_DEFAULT_MAX_COMMENT_THREAD_DEPTH` | `0` (unlimited) |

All of them are documented in the README, `.env.example` and `app.json`.

### Settings codes

Settings codes (Settings → export / restore) now carry a format revision:

- **New codes** start with a marker byte and are revisioned. A code exported before a setting existed restores that setting to its default.
- **Old codes** from upstream Redlib still restore. Settings they don't contain get their defaults.
- All settings added in this fork belong to revision 2.

Codes exported from this fork can't be restored on upstream Redlib.

## Dependencies and networking

Upgraded in the hope of reducing Reddit's TLS/SSL and fingerprinting errors (rate limits, failed token refreshes, broken media):

- **wreq** `6.0.0-rc.28` → `0.16.1` and **wreq-util** `3.0.0-rc.10` → `0.2.0`, the stable line of the HTTP client that emulates browser TLS and HTTP/2 fingerprints. The BoringSSL bindings move from `boring2 5.0.0-alpha.13` to `btls 0.5.6`. 0.16.1 also applies the total timeout to the response body, uses the configured timer for connect timeouts, and no longer compresses range requests.
- The emulated browsers stay the same: Chrome 145 and Firefox 147, each on Android or Windows, chosen at random. Their JA4, Akamai HTTP/2 and peetprint fingerprints were checked against tls.peet.ws after every upgrade.
- **hyper** `0.14` → `1.x` for the server, with graceful shutdown.
- **hls.js** `1.5.1` → `1.7.3`.
- All other Rust dependencies are updated (askama 0.16, cached 4, revision 0.30, toml 1.0, brotli 9, base64 0.23, …). The minimum Rust version is 1.98.

## Removals

- **quay.io image** and the **Alpine image** (`Dockerfile.alpine`) are replaced by the multi-arch distroless image on GHCR.
- **Docker `HEALTHCHECK`** in `compose.yaml` and `compose.dev.yaml`: it used `wget`, which the image doesn't have. Probe `/healthz` instead.
- **Release binary, Rust test and pull-request workflows** are disabled (`*.disabled`). Only the container build and lint workflows run.

## Development

- Formatting is enforced with pre-commit and a Lint workflow: `cargo fmt`, [djhtml](https://github.com/rtts/djhtml) for templates, Prettier for CSS and JS, plus whitespace and line-ending checks. Run `pre-commit install` once.
- `.editorconfig`: tabs for Rust, 4 spaces for templates, CSS and JS, 2 spaces for YAML and JSON.
- CI builds each platform in its own job, with the Cargo registry and `target/` cached between runs.
- The Docker build context is an allowlist (`.dockerignore`); the commit hash comes in as the `GIT_HASH` build argument.
