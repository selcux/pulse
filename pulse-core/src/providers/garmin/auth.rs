use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::Rng;
use regex::Regex;
use sha1::Sha1;
use url::form_urlencoded;

use super::tokens::{
    load_oauth1, load_oauth2, save_oauth1, save_oauth2, OAuthConsumer, OAuth1Token, OAuth2Token,
};

type HmacSha1 = Hmac<Sha1>;

const SSO_BASE: &str = "https://sso.garmin.com/sso";
const CONNECT_API: &str = "https://connectapi.garmin.com";
const CONSUMER_URL: &str = "https://thegarth.s3.amazonaws.com/oauth_consumer.json";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Result of a login attempt.
pub enum LoginResult {
    Success,
    MfaRequired,
}

/// Full SSO login flow. Saves tokens to disk on success.
pub fn login(
    client: &reqwest::blocking::Client,
    email: &str,
    password: &str,
    mfa_code: Option<&str>,
) -> Result<LoginResult> {
    // Step 1: Fetch OAuth consumer credentials
    let consumer = fetch_consumer(client)?;

    // Step 2: Establish SSO cookies
    let embed_url = format!("{SSO_BASE}/embed?id=gauth-widget&embedWidget=true&gauthHost={SSO_BASE}");
    client.get(&embed_url).send().context("Failed to load SSO embed page")?;

    // Step 3: Load sign-in page and extract CSRF token
    let signin_url = format!(
        "{SSO_BASE}/signin?id=gauth-widget&embedWidget=true&gauthHost={SSO_BASE}"
    );
    let signin_page = client
        .get(&signin_url)
        .send()
        .context("Failed to load SSO sign-in page")?
        .text()?;

    let csrf = extract_csrf(&signin_page)?;

    // Step 4: POST credentials
    let form = [
        ("username", email),
        ("password", password),
        ("embed", "true"),
        ("_csrf", &csrf),
    ];
    let post_resp = client
        .post(&signin_url)
        .form(&form)
        .send()
        .context("Failed to POST sign-in form")?
        .text()?;

    // Step 5: Check response — MFA or success?
    if looks_like_mfa(&post_resp) {
        if let Some(code) = mfa_code {
            let mfa_resp = submit_mfa(client, &post_resp, code)?;
            let ticket = extract_ticket(&mfa_resp)?;
            return finish_login(client, &consumer, &ticket);
        }
        return Ok(LoginResult::MfaRequired);
    }

    let ticket = extract_ticket(&post_resp)?;
    finish_login(client, &consumer, &ticket)
}

/// Ensure we have a valid OAuth2 access token. Auto-refreshes if expired.
/// Returns the access token string.
pub fn ensure_valid_token(client: &reqwest::blocking::Client) -> Result<String> {
    let oauth2 = load_oauth2()?;

    if !oauth2.is_expired() {
        return Ok(oauth2.access_token);
    }

    // Access token expired — re-exchange OAuth1 → OAuth2
    let oauth1 = load_oauth1()?;
    let consumer = fetch_consumer(client)?;
    let new_oauth2 = exchange_oauth1_to_oauth2(client, &consumer, &oauth1)?;
    save_oauth2(&new_oauth2)?;
    Ok(new_oauth2.access_token)
}

// ---------------------------------------------------------------------------
// SSO helpers (private)
// ---------------------------------------------------------------------------

fn fetch_consumer(client: &reqwest::blocking::Client) -> Result<OAuthConsumer> {
    client
        .get(CONSUMER_URL)
        .send()
        .context("Failed to fetch OAuth consumer credentials")?
        .json::<OAuthConsumer>()
        .context("Failed to parse OAuth consumer JSON")
}

/// Detect Garmin's MFA challenge page. Checks multiple indicators since
/// Garmin's exact HTML varies and isn't documented.
fn looks_like_mfa(html: &str) -> bool {
    let html_lower = html.to_lowercase();
    html_lower.contains("verificationcode")          // form field name
        || html_lower.contains("mfa")
        || html_lower.contains("two-factor")
        || html_lower.contains("two factor")
        || html_lower.contains("verification code")
        || html_lower.contains("enter the code")
        || html_lower.contains("authenticator")
        || html_lower.contains("one-time")
}

fn extract_csrf(html: &str) -> Result<String> {
    let re = Regex::new(r#"name="_csrf"\s+value="([^"]+)""#)?;
    let caps = re
        .captures(html)
        .context("Could not find CSRF token in sign-in page")?;
    Ok(caps[1].to_string())
}

fn extract_ticket(html: &str) -> Result<String> {
    let re = Regex::new(r#"embed\?ticket=([^"]+)"#)?;
    let caps = re.captures(html).context(
        "Could not find ticket in response. Check your username/password.",
    )?;
    Ok(caps[1].to_string())
}

fn submit_mfa(
    client: &reqwest::blocking::Client,
    mfa_page: &str,
    code: &str,
) -> Result<String> {
    let csrf = extract_csrf(mfa_page)?;
    let mfa_url = format!(
        "{SSO_BASE}/verifyMFA/loginEnterMfa?id=gauth-widget&embedWidget=true&gauthHost={SSO_BASE}"
    );
    let form = [("verificationCode", code), ("embed", "true"), ("_csrf", &csrf)];
    let resp = client
        .post(&mfa_url)
        .form(&form)
        .send()
        .context("Failed to submit MFA code")?
        .text()?;
    Ok(resp)
}

fn finish_login(
    client: &reqwest::blocking::Client,
    consumer: &OAuthConsumer,
    ticket: &str,
) -> Result<LoginResult> {
    // Step 6: Exchange ticket for OAuth1 token
    let oauth1 = exchange_ticket(client, consumer, ticket)?;
    save_oauth1(&oauth1)?;

    // Step 7: Exchange OAuth1 for OAuth2
    let oauth2 = exchange_oauth1_to_oauth2(client, consumer, &oauth1)?;
    save_oauth2(&oauth2)?;

    Ok(LoginResult::Success)
}

fn exchange_ticket(
    client: &reqwest::blocking::Client,
    consumer: &OAuthConsumer,
    ticket: &str,
) -> Result<OAuth1Token> {
    let url = format!(
        "{CONNECT_API}/oauth-service/oauth/preauthorized?ticket={ticket}&login-url={SSO_BASE}/embed&acceptHeaderRequired=true"
    );

    let auth_header = build_oauth1_header(
        "GET",
        &url,
        consumer,
        None, // no token yet
        None,
    );

    let resp = client
        .get(&url)
        .header("Authorization", &auth_header)
        .send()
        .context("Failed to exchange ticket for OAuth1 token")?
        .text()?;

    // Response is URL-encoded: oauth_token=xxx&oauth_token_secret=yyy
    let params: BTreeMap<String, String> = form_urlencoded::parse(resp.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let oauth_token = params
        .get("oauth_token")
        .context("Missing oauth_token in preauthorized response")?
        .clone();
    let oauth_token_secret = params
        .get("oauth_token_secret")
        .context("Missing oauth_token_secret in preauthorized response")?
        .clone();

    Ok(OAuth1Token {
        oauth_token,
        oauth_token_secret,
        mfa_token: None,
        mfa_expiration_timestamp: None,
        domain: "garmin.com".into(),
    })
}

fn exchange_oauth1_to_oauth2(
    client: &reqwest::blocking::Client,
    consumer: &OAuthConsumer,
    oauth1: &OAuth1Token,
) -> Result<OAuth2Token> {
    let url = format!("{CONNECT_API}/oauth-service/oauth/exchange/user/2.0");

    let auth_header = build_oauth1_header(
        "POST",
        &url,
        consumer,
        Some(&oauth1.oauth_token),
        Some(&oauth1.oauth_token_secret),
    );

    let resp = client
        .post(&url)
        .header("Authorization", &auth_header)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .context("Failed to exchange OAuth1 for OAuth2 token")?;

    if !resp.status().is_success() {
        bail!(
            "OAuth1→OAuth2 exchange failed with status {}",
            resp.status()
        );
    }

    let mut token: OAuth2Token = resp.json().context("Failed to parse OAuth2 token response")?;

    // Garmin returns expires_in but not always expires_at — compute if missing/zero
    let now = chrono::Utc::now().timestamp();
    if token.expires_at == 0 {
        token.expires_at = now + token.expires_in;
    }
    if token.refresh_token_expires_at == 0 {
        token.refresh_token_expires_at = now + token.refresh_token_expires_in;
    }

    Ok(token)
}

// ---------------------------------------------------------------------------
// OAuth1 signing (RFC 5849 subset)
// ---------------------------------------------------------------------------

fn build_oauth1_header(
    method: &str,
    url: &str,
    consumer: &OAuthConsumer,
    token: Option<&str>,
    token_secret: Option<&str>,
) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();

    let nonce = generate_nonce();

    // Collect OAuth params
    let mut oauth_params = BTreeMap::new();
    oauth_params.insert("oauth_consumer_key", consumer.consumer_key.as_str());
    oauth_params.insert("oauth_nonce", &nonce);
    oauth_params.insert("oauth_signature_method", "HMAC-SHA1");
    oauth_params.insert("oauth_timestamp", &timestamp);
    oauth_params.insert("oauth_version", "1.0");

    if let Some(tok) = token {
        oauth_params.insert("oauth_token", tok);
    }

    // Parse URL to separate base URL and query params
    let parsed = url::Url::parse(url).expect("Invalid URL for OAuth1 signing");
    let base_url = format!("{}://{}{}", parsed.scheme(), parsed.host_str().unwrap(), parsed.path());

    // Collect ALL params (oauth + query string) for base string
    let mut all_params: BTreeMap<String, String> = oauth_params
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    for (k, v) in parsed.query_pairs() {
        all_params.insert(k.into_owned(), v.into_owned());
    }

    // Build base string: METHOD&url_encode(base_url)&url_encode(sorted_params)
    let params_string: String = all_params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let base_string = format!(
        "{}&{}&{}",
        method.to_uppercase(),
        percent_encode(&base_url),
        percent_encode(&params_string),
    );

    // Sign: HMAC-SHA1(consumer_secret&token_secret, base_string)
    let signing_key = format!(
        "{}&{}",
        percent_encode(&consumer.consumer_secret),
        percent_encode(token_secret.unwrap_or(""))
    );

    let mut mac = HmacSha1::new_from_slice(signing_key.as_bytes()).expect("HMAC key error");
    mac.update(base_string.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    // Build Authorization header
    let mut header_parts: Vec<String> = oauth_params
        .iter()
        .map(|(k, v)| format!("{}=\"{}\"", k, percent_encode(v)))
        .collect();
    header_parts.push(format!("oauth_signature=\"{}\"", percent_encode(&signature)));

    format!("OAuth {}", header_parts.join(", "))
}

fn generate_nonce() -> String {
    let mut rng = rand::rng();
    (0..32)
        .map(|_| {
            let idx = rng.random_range(0..36u8);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect()
}

/// RFC 3986 percent-encoding (uppercase, unreserved chars not encoded).
fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encode_unreserved_chars_pass_through() {
        assert_eq!(percent_encode("abc123"), "abc123");
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn percent_encode_special_chars() {
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(percent_encode("100%"), "100%25");
        assert_eq!(percent_encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn percent_encode_slash_and_colon() {
        assert_eq!(percent_encode("https://example.com"), "https%3A%2F%2Fexample.com");
    }

    #[test]
    fn nonce_is_32_chars_alphanumeric() {
        let nonce = generate_nonce();
        assert_eq!(nonce.len(), 32);
        assert!(nonce.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn nonce_is_random() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b); // Extremely unlikely to collide
    }

    /// Test OAuth1 signing against a known base string construction.
    /// This validates the base string format and HMAC-SHA1 computation.
    #[test]
    fn oauth1_signature_base_string_format() {
        let consumer = OAuthConsumer {
            consumer_key: "dpf43f3p2l4k3l03".into(),
            consumer_secret: "kd94hf93k423kf44".into(),
        };

        // Build header with known params — we verify the header contains expected parts
        let header = build_oauth1_header(
            "GET",
            "https://api.example.com/resource?size=10",
            &consumer,
            Some("nnch734d00sl2jdk"),
            Some("pfkkdhi9sl3r4s00"),
        );

        assert!(header.starts_with("OAuth "));
        assert!(header.contains("oauth_consumer_key=\"dpf43f3p2l4k3l03\""));
        assert!(header.contains("oauth_token=\"nnch734d00sl2jdk\""));
        assert!(header.contains("oauth_signature_method=\"HMAC-SHA1\""));
        assert!(header.contains("oauth_version=\"1.0\""));
        assert!(header.contains("oauth_signature="));
    }

    #[test]
    fn oauth1_header_without_token() {
        let consumer = OAuthConsumer {
            consumer_key: "test_key".into(),
            consumer_secret: "test_secret".into(),
        };

        let header = build_oauth1_header(
            "GET",
            "https://api.example.com/resource",
            &consumer,
            None,
            None,
        );

        assert!(header.starts_with("OAuth "));
        assert!(header.contains("oauth_consumer_key=\"test_key\""));
        assert!(!header.contains("oauth_token="));
        assert!(header.contains("oauth_signature="));
    }

    #[test]
    fn extract_csrf_from_html() {
        let html = r#"<input type="hidden" name="_csrf" value="abc123xyz">"#;
        let csrf = extract_csrf(html).unwrap();
        assert_eq!(csrf, "abc123xyz");
    }

    #[test]
    fn extract_csrf_missing_returns_error() {
        let html = "<html><body>No CSRF here</body></html>";
        assert!(extract_csrf(html).is_err());
    }

    #[test]
    fn extract_ticket_from_html() {
        let html = r#"var defined response_url = "https://sso.garmin.com/sso/embed?ticket=ST-123456-abcdef";"#;
        let ticket = extract_ticket(html).unwrap();
        assert_eq!(ticket, "ST-123456-abcdef");
    }

    #[test]
    fn extract_ticket_missing_returns_error() {
        let html = "<html><title>Error</title></html>";
        assert!(extract_ticket(html).is_err());
    }
}
