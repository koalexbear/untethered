//! Sign-in through the system browser: homeserver single sign-on
//! (`m.login.sso`) and OAuth 2.0 with the homeserver's authentication service
//! (MAS, MSC3861). The browser comes back to a one-shot listener on
//! 127.0.0.1 (RFC 8252 loopback redirect), so no URL scheme is registered.
//! A web page can't listen, so the web client signs in with a password until
//! these flows get a redirect back to the page.
use super::{BrowserMethod, Failure, LoginOptions, session};
use matrix_sdk::{Client, ruma::api::client::session::get_login_types::v3::LoginType};
#[cfg(not(target_arch = "wasm32"))]
use matrix_sdk::{
    authentication::oauth::{
        ClientRegistrationData,
        registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
    },
    ruma::serde::Raw,
    utils::UrlOrQuery,
};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

/// Opens a sign-in page. Tests replace the system browser.
pub type Opener = Arc<dyn Fn(Url) + Send + Sync>;

/// The project page shown by OAuth 2.0 consent screens.
#[cfg(not(target_arch = "wasm32"))]
const CLIENT_URI: &str = "https://github.com/koalexbear/untethered";
/// How long a browser sign-in may take before it is abandoned.
#[cfg(not(target_arch = "wasm32"))]
const PATIENCE: Duration = Duration::from_secs(15 * 60);

/// The user's browser, started without a shell.
#[cfg(target_arch = "wasm32")]
pub fn system() -> Opener {
    Arc::new(|_| {})
}
/// The user's browser, started without a shell.
#[cfg(not(target_arch = "wasm32"))]
pub fn system() -> Opener {
    Arc::new(|url: Url| {
        #[cfg(target_os = "linux")]
        let child = std::process::Command::new("xdg-open")
            .arg(url.as_str())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        #[cfg(windows)]
        let child = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url.as_str()])
            .spawn();
        #[cfg(not(any(target_os = "linux", windows)))]
        let child: std::io::Result<std::process::Child> =
            Err(std::io::ErrorKind::Unsupported.into());
        if let Ok(mut child) = child {
            std::thread::spawn(move || child.wait());
        }
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn open(opener: &Opener, url: Url) -> Result<(), Failure> {
    // Only web pages: never hand a server-chosen file: or app: URL to the OS.
    if !matches!(url.scheme(), "https" | "http") {
        return Err(Failure::Failed);
    }
    opener(url);
    Ok(())
}

pub async fn options(homeserver: &str) -> Result<LoginOptions, Failure> {
    let client = Client::builder()
        .server_name_or_homeserver_url(homeserver)
        .build()
        .await
        .map_err(|_| Failure::Unreachable)?;
    let mut options = LoginOptions {
        homeserver: client.homeserver().to_string(),
        oauth: client.oauth().server_metadata().await.is_ok(),
        ..LoginOptions::default()
    };
    let types = client
        .matrix_auth()
        .get_login_types()
        .await
        .map_err(|_| Failure::Unreachable)?;
    for flow in types.flows {
        match flow {
            LoginType::Password(_) => options.password = true,
            LoginType::Sso(sso) => {
                options.sso = true;
                options
                    .providers
                    .extend(sso.identity_providers.into_iter().map(|p| (p.id, p.name)));
            }
            _ => {}
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        options.sso = false;
        options.oauth = false;
        options.providers.clear();
    }
    Ok(options)
}

#[cfg(target_arch = "wasm32")]
pub async fn login(
    _profile: &session::Profile,
    _homeserver: &str,
    _method: BrowserMethod,
    _opener: Opener,
) -> Result<Client, Failure> {
    Err(Failure::Unavailable)
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn login(
    profile: &session::Profile,
    homeserver: &str,
    method: BrowserMethod,
    opener: Opener,
) -> Result<Client, Failure> {
    let (builder, secret) = session::fresh_store(profile).await?;
    let result = async {
        let client = builder
            .server_name_or_homeserver_url(homeserver)
            .build()
            .await
            .map_err(|_| Failure::Unreachable)?;
        if client.available_sliding_sync_versions().await.is_empty() {
            return Err(Failure::NoSlidingSync);
        }
        let flow = async {
            match method {
                BrowserMethod::Sso { provider } => sso(&client, provider, &opener).await,
                BrowserMethod::OAuth => oauth(&client, &opener).await,
            }
        };
        tokio::time::timeout(PATIENCE, flow)
            .await
            .map_err(|_| Failure::Cancelled)??;
        session::save(profile, &client, &secret).await?;
        Ok(client)
    }
    .await;
    if result.is_err() {
        session::forget(profile).await;
    }
    result
}

#[cfg(not(target_arch = "wasm32"))]
async fn sso(client: &Client, provider: Option<String>, opener: &Opener) -> Result<(), Failure> {
    let (listener, redirect) = listen().await?;
    let url = client
        .matrix_auth()
        .get_sso_login_url(redirect.as_str(), provider.as_deref())
        .await
        .map_err(|_| Failure::Failed)?;
    open(opener, Url::parse(&url).map_err(|_| Failure::Failed)?)?;
    let back = redirected(listener, &redirect).await?;
    let token = back
        .query_pairs()
        .find(|(key, _)| key == "loginToken")
        .map(|(_, value)| value.into_owned())
        .ok_or(Failure::Forbidden)?;
    client
        .matrix_auth()
        .login_token(&token)
        .initial_device_display_name("Untethered")
        .await
        .map_err(|e| {
            if e.client_api_error_kind().is_some() {
                Failure::Forbidden
            } else {
                Failure::Unreachable
            }
        })?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn oauth(client: &Client, opener: &Opener) -> Result<(), Failure> {
    let (listener, redirect) = listen().await?;
    let mut metadata = ClientMetadata::new(
        ApplicationType::Native,
        vec![OAuthGrantType::AuthorizationCode {
            redirect_uris: vec![redirect.clone()],
        }],
        Localized::new(Url::parse(CLIENT_URI).expect("valid client URI"), []),
    );
    metadata.client_name = Some(Localized::new("Untethered".to_owned(), []));
    let registration =
        ClientRegistrationData::new(Raw::new(&metadata).map_err(|_| Failure::Failed)?);
    let data = client
        .oauth()
        .login(redirect.clone(), None, Some(registration), None)
        .build()
        .await
        .map_err(|_| Failure::Failed)?;
    open(opener, data.url)?;
    let back = redirected(listener, &redirect).await?;
    client
        .oauth()
        .finish_login(UrlOrQuery::Url(back))
        .await
        .map_err(|_| Failure::Forbidden)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn listen() -> Result<(TcpListener, Url), Failure> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|_| Failure::Failed)?;
    let port = listener.local_addr().map_err(|_| Failure::Failed)?.port();
    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|_| Failure::Failed)?;
    Ok((listener, url))
}

/// Wait for the browser to come back to `base` and answer it.
#[cfg(not(target_arch = "wasm32"))]
async fn redirected(listener: TcpListener, base: &Url) -> Result<Url, Failure> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|_| Failure::Failed)?;
        let mut request = Vec::new();
        let mut chunk = [0u8; 2048];
        let read = tokio::time::timeout(Duration::from_secs(10), async {
            while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 16 * 1024 {
                let n = stream.read(&mut chunk).await.ok()?;
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
            }
            Some(())
        })
        .await;
        let target = (read.is_ok())
            .then(|| String::from_utf8_lossy(&request).into_owned())
            .and_then(|text| {
                let line = text.lines().next()?.to_owned();
                let mut parts = line.split_whitespace();
                (parts.next() == Some("GET")).then(|| parts.next().map(str::to_owned))?
            });
        let Some(target) = target.filter(|t| t.starts_with("/?")) else {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            continue;
        };
        let page = "<!doctype html><meta charset=utf-8><title>Untethered</title>\
            <p>You can close this tab and return to Untethered.</p>";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                )
                .as_bytes(),
            )
            .await;
        return base.join(&target).map_err(|_| Failure::Failed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn loopback_listener_returns_the_redirect_and_ignores_other_paths() {
        let (listener, base) = listen().await.unwrap();
        let waiting = base.clone();
        let back = tokio::spawn(async move { redirected(listener, &waiting).await });
        let http = reqwest::Client::new();
        let icon = http.get(format!("{base}favicon.ico")).send().await.unwrap();
        assert_eq!(icon.status(), 404);
        let page = http
            .get(format!("{base}?code=abc&state=xyz"))
            .send()
            .await
            .unwrap();
        assert!(page.text().await.unwrap().contains("return to Untethered"));
        let url = back.await.unwrap().unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.query(), Some("code=abc&state=xyz"));
    }

    #[test]
    fn only_web_pages_reach_the_browser() {
        let opened = Arc::new(AtomicUsize::new(0));
        let count = opened.clone();
        let opener: Opener = Arc::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        });
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "x-scheme-handler:open",
        ] {
            assert!(open(&opener, Url::parse(bad).unwrap()).is_err(), "{bad}");
        }
        assert_eq!(opened.load(Ordering::SeqCst), 0);
        open(&opener, Url::parse("https://example.org/login").unwrap()).unwrap();
        assert_eq!(opened.load(Ordering::SeqCst), 1);
    }
}
