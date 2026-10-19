# Spike: Udemy login through an in-app webview

Question: can Lumen log in to the owner's personal Udemy account (`www.udemy.com`) inside a Tauri webview and read the
session cookies, instead of asking the user to paste them?

## What exists

`udemy_login_webview` reuses the generic webview login of Skool and Hotmart (`capture_webview_login` in
`src-tauri/src/lib.rs`): it opens a window labelled `udemy-login` at `https://www.udemy.com/join/login-popup/`, polls
`WebviewWindow::cookies_for_url` (HttpOnly cookies included) inside `spawn_blocking`, and as soon as an `access_token`
cookie appears it builds the cookie header, validates it with a minimal courses request (`page_size=1`), stores it in
the app data folder (`udemy-session`, mode 0600 on Unix) and wipes the webview profile. The window has no IPC
capability. Outcomes: `login_cancelled` (window closed), `login_timeout` (5 minutes), `session_expired` or
`bot_challenge` when Udemy rejects the cookie.

## What could be verified without live access

- The cookie API works on Linux (WebKitGTK): see `skool-webview-login.md`, same mechanism, tested there with a local
  page. Nothing Udemy-specific was added to that mechanism.
- The pure parts are unit tested: extracting `access_token` from a cookie header or from the jar
  (`header_from_jar`), refusing a jar without it, never logging cookie values, and persisting only after validation
  (mock server).

## Not verified (needs the owner's real account)

- That the web login really sets an `access_token` cookie readable by the webview. This is a HYPOTHESIS; the
  diagnostic's `session` step reports the real status of the first authenticated request.
- Whether Udemy shows a captcha or bot challenge inside an embedded webview (likely for a non-standard user agent).
  The command just tries; a challenge ends as `login_timeout`/`login_cancelled`, and the client reports a 403 challenge
  page as the typed error `bot_challenge`. Fallback: paste the `Cookie` header with `udemy_login_cookies`.
- Social login (Google/Apple/Facebook) popups and 2FA inside the webview.
- Windows / WebView2 (the same deadlock caveat as Skool applies; the jar is read inside `spawn_blocking`).
