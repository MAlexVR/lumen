# Spike: Hotmart login

Question: what credential does Hotmart need, and can Lumen obtain it from an in-app webview instead of asking the user
to paste it?

## What the credential is

Taken from the previous Go implementation (its Hotmart login and API headers) and its UI strings
(the token label and hint):

- The user supplies the value of the **`hmVlcIntegration` cookie** of `https://consumer.hotmart.com` (in the browser: log in, open
  DevTools, go to Application (Storage in Firefox), Cookies, select `https://consumer.hotmart.com` and copy the
  value of the `hmVlcIntegration` row).
- That value is used as an OAuth bearer token: `Authorization: Bearer <value>` on every Hotmart API call.
- An optional full `Cookie` header can be supplied as well; the Go code sends it when present (it is not required by
  the code paths we know).
- Validation in the Go code: `GET api-sec-vlc.hotmart.com/security/oauth/check_token?token=<value>`; when that fails,
  `GET api-hub.cb.hotmart.com/club-drive-api/rest/v2/purchase/?archived=UNARCHIVED` must return a `data` array.
- There is **no token refresh** anywhere in the Go code. An expired token surfaces as HTTP 401/403 and the user has to
  log in again (Lumen reports `session_expired`).

## Can a webview yield it?

Probably yes, but this is **not verified against the live site**.

Evidence for:

- The cookie lives in the cookie store of `consumer.hotmart.com` after a normal browser login, so an embedded browser
  engine that completes the same login holds it too.
- `WebviewWindow::cookies_for_url` returns HttpOnly and Secure cookies on WebKitGTK (observed in
  `docs/spikes/skool-webview-login.md`), so it does not matter whether `hmVlcIntegration` is HttpOnly (the DevTools
  column that tells is easy to miss).

Not verified (cannot be done without a Hotmart account and network access from this environment):

- That the Hotmart SSO (`sso.hotmart.com`, social logins, 2FA, the AWS WAF visible as `aws-waf-token`) completes
  inside the embedded engine.
- The exact cookie domain and whether it appears as soon as the login finishes or only after the consumer page loads.
- Token lifetime.
- Windows / WebView2.

## What was built

`hotmart_login_webview` is offered **next to** the paste path, never instead of it:

1. Opens a window labelled `hotmart-login` at `https://consumer.hotmart.com/` (it redirects to the SSO when logged
   out). The window is not covered by any capability, so the remote page cannot call Tauri commands.
2. Polls `cookies_for_url("https://consumer.hotmart.com/")` every second for up to 5 minutes until `hmVlcIntegration`
   is non-empty. Closing the window returns `login_cancelled`, the timeout `login_timeout`.
3. Wipes the webview's browsing data and closes it, then validates the token against Hotmart exactly like the paste
   path. Only a token Hotmart accepts is stored.

If the webview login does not work on a machine, `hotmart_login_credentials(token, cookies?)` is the fallback and is
the path the owner should test first.

## Where the session lives

`<app data dir>/hotmart-session` (Linux: `~/.local/share/com.malexvr.lumen/`), a small JSON document
`{"token": "...", "cookies": null | "..."}`. It is outside the repository and the library folder, never logged, and on
Unix created with mode 0600 through a private temporary file and renamed (same store as Skool). Windows relies on the
private `%APPDATA%` ACL (unverified).
