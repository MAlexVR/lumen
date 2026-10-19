# Spike: Skool login through an in-app webview

Question: can Lumen log in to Skool inside a Tauri webview (a real browser engine, which the CloudFront WAF accepts) and
read the session cookies, instead of asking the user to paste them?

## Answer

Yes, the Tauri API needed exists and works on Linux. Tauri 2.12.4 (see `Cargo.lock`) exposes:

- `WebviewWindow::cookies_for_url(Url) -> Result<Vec<Cookie>>` and `WebviewWindow::cookies()`: the runtime cookie store,
  **HttpOnly and Secure cookies included**.
- `WebviewWindow::clear_all_browsing_data()`: wipes the webview profile.

### Observed (Linux, WebKitGTK, GNOME Wayland, 2026-10)

A throwaway program (not committed) served a local page on `127.0.0.1` that sent
`Set-Cookie: auth_token=...; HttpOnly` and `Set-Cookie: aws-waf-token=...`, opened it in a `WebviewWindowBuilder`
window with an external URL, and polled `cookies_for_url` from a separate thread every 500 ms:

```
t=502ms cookies_for_url -> ["auth_token(len=18,httponly=Some(true))", "aws-waf-token(len=3,httponly=Some(false))"]
clear_all_browsing_data -> Ok(())
after clear: Ok(0)
```

So the HttpOnly `auth_token` is readable, polling works from a non-main thread, and the profile can be wiped.

### Not verified

- A real Skool login (credentials, 2FA, Google/Apple sign-in popups) inside the webview, and whether the WAF lets the
  embedded engine through. The WAF blocks non-browser clients; a real WebKitGTK/WebView2 engine is expected to pass but
  this was not tried against skool.com.
- Windows / WebView2. Tauri documents that `cookies()` can deadlock on Windows when called from a synchronous command
  or event handler. Lumen avoids it: `skool_login_webview` is an `async` command and reads the jar inside
  `spawn_blocking`. Still unverified on Windows.
- macOS (not a target).

## How the app uses it (`src-tauri/src/lib.rs`)

1. `skool_login_webview` opens a window labelled `skool-login` at `https://www.skool.com/login`. It is not covered by
   any capability, so the remote page has no access to Tauri commands.
2. Every second (up to 5 minutes) it reads `cookies_for_url("https://www.skool.com/login")` and stops when `auth_token`
   is present. Closing the window early returns `login_cancelled`; the timeout returns `login_timeout`.
3. It builds the `Cookie` header from the jar, calls `clear_all_browsing_data()` and closes the window, so no second copy
   of the session stays in the webview profile.
4. The header goes through the same validation as the paste path (`GET api2.skool.com/self/groups`); only a header that
   Skool accepts is stored.

The paste path (`skool_login_cookies`) stays as the fallback if the embedded login does not work on a machine.

## Where the session lives

`<app data dir>/skool-session` (Linux: `~/.local/share/com.malexvr.lumen/`), outside the repository and the library
folder, never logged. On Unix the file is created with mode 0600 through a private temporary file and renamed. On
Windows the folder is `%APPDATA%\com.malexvr.lumen`, private to the user through its inherited ACL; no extra ACL is set
(unverified on Windows; DPAPI encryption is a possible hardening).
