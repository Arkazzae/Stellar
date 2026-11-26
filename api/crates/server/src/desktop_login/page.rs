//! The two small pages the system browser is shown: a notice, and the
//! confirmation with the button that approves.
//!
//! The pages are self-contained. The content security policy forbids scripts, outside
//! resource, every frame that would hold the page and every form that would
//! send it elsewhere. Every value that is put into the markup passes through
//! [`encode`].

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;

/// What the browser may do with the page: nothing but style it.
const POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

/// The page of a notice: a heading and a sentence.
pub fn notice(language: &str, heading: &str, body: &str, status: StatusCode) -> Response {
    let heading = encode(heading);
    let body = encode(body);
    let html = format!(
        "<!doctype html>
<html lang=\"{language}\">
<head>
  <meta charset=\"utf-8\">
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
  <title>{heading}</title>
  <style>
    :root {{ color-scheme: dark; font-family: Inter, system-ui, sans-serif; }}
    body {{ min-height: 100vh; margin: 0; display: grid; place-items: center; background: #101112; color: #f3f1ec; }}
    main {{ width: min(34rem, calc(100% - 2rem)); padding: 2rem; border: 1px solid #343638; border-radius: 1rem; background: #191b1d; }}
    h1 {{ margin: 0 0 .75rem; font-size: 1.5rem; }}
    p {{ margin: 0; color: #b8b9b7; line-height: 1.55; }}
  </style>
</head>
<body><main><h1>{heading}</h1><p>{body}</p></main></body>
</html>"
    );
    html_response(html, status)
}

/// What the confirmation page says and where its form goes.
pub struct Confirmation<'a> {
    pub language: &'a str,
    pub heading: &'a str,
    pub body: &'a str,
    pub button: &'a str,
    /// Where the form posts.
    pub action: &'a str,
    /// What binds the form to this person, this browser session and this
    /// request.
    pub token: &'a str,
}

/// The page that asks the person to approve, which never approves by itself.
pub fn confirmation(page: &Confirmation<'_>) -> Response {
    let language = page.language;
    let heading = encode(page.heading);
    let body = encode(page.body);
    let button = encode(page.button);
    let action = encode(page.action);
    let token = encode(page.token);
    let html = format!(
        "<!doctype html>
<html lang=\"{language}\">
<head>
  <meta charset=\"utf-8\">
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
  <title>{heading}</title>
  <style>
    :root {{ color-scheme: dark; font-family: Inter, system-ui, sans-serif; }}
    body {{ min-height: 100vh; margin: 0; display: grid; place-items: center; background: #101112; color: #f3f1ec; }}
    main {{ width: min(34rem, calc(100% - 2rem)); padding: 2rem; border: 1px solid #343638; border-radius: 1rem; background: #191b1d; }}
    h1 {{ margin: 0 0 .75rem; font-size: 1.5rem; }}
    p {{ margin: 0 0 1.5rem; color: #b8b9b7; line-height: 1.55; }}
    button {{ padding: .8rem 1.1rem; border: 0; border-radius: .5rem; background: #d6a554; color: #17130d; font: inherit; font-weight: 700; cursor: pointer; }}
  </style>
</head>
<body><main>
  <h1>{heading}</h1>
  <p>{body}</p>
  <form method=\"post\" action=\"{action}\">
    <input type=\"hidden\" name=\"token\" value=\"{token}\">
    <button type=\"submit\">{button}</button>
  </form>
</main></body>
</html>"
    );
    html_response(html, StatusCode::OK)
}

/// The page with the headers that keep it where it was shown: out of caches,
/// out of other sites' frames, and without a referrer on anything that leads
/// away from the instance. The policy is `same-origin`, not `no-referrer`: a
/// browser writes `Origin: null` on a form it posts from a page that sends no
/// referrer at all, and the approval takes only a post that names this origin.
fn html_response(html: String, status: StatusCode) -> Response {
    let mut response = Response::new(Body::from(html));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(POLICY),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// Escapes text before inserting it into markup:
/// the five characters that mean something in markup, and every character of
/// Latin-1 above the space and every one outside the Basic Multilingual Plane
/// as a number, so that the page is plain ASCII wherever that matters.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            c if (160..256).contains(&(c as u32)) || (c as u32) >= 0x1_0000 => {
                out.push_str(&format!("&#{};", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn text(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn markup_is_escaped_and_latin_one_and_astral_characters_become_numbers() {
        assert_eq!(
            encode("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
        // 0xA0 is the first and 0xFF the last character that becomes a number;
        // 0x9F and 0x100 stay.
        assert_eq!(
            encode("\u{9f}\u{a0}ó\u{ff}\u{100}ż"),
            "\u{9f}&#160;&#243;&#255;\u{100}ż"
        );
        assert_eq!(encode("a😀b"), "a&#128512;b");
        // Control characters and right-to-left text are not the encoder's
        // business: they are inert in element content.
        assert_eq!(
            encode("\u{0}\u{7}\u{202e}שלום"),
            "\u{0}\u{7}\u{202e}שלום"
        );
    }

    #[test]
    fn markup_escaping_changes_exactly_the_expected_code_points() {
        let mut changed: Vec<u32> = vec![34, 38, 39, 60, 62];
        changed.extend(160..=255);
        for code in (0..=0x10FFFF).filter(|code| !(0xD800..=0xDFFF).contains(code)) {
            let c = char::from_u32(code).unwrap();
            let text = c.to_string();
            let wanted = if changed.contains(&code) || code >= 0x10000 {
                encode(&text) != text
            } else {
                encode(&text) == text
            };
            assert!(wanted, "{code}");
            if code >= 0x10000 || (160..=255).contains(&code) {
                assert_eq!(encode(&text), format!("&#{code};"));
            }
        }
    }

    #[tokio::test]
    async fn the_notice_renders_the_expected_html() {
        let response = notice(
            "pl",
            "Zalogowano do <Stellarr>",
            "Możesz zamknąć tę kartę",
            StatusCode::GONE,
        );
        assert_eq!(response.status(), StatusCode::GONE);
        let headers = response.headers().clone();
        assert_eq!(headers["content-type"], "text/html; charset=utf-8");
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(headers["pragma"], "no-cache");
        assert_eq!(headers["referrer-policy"], "same-origin");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["content-security-policy"], POLICY);
        assert!(!headers.contains_key("x-frame-options"));
        let page = text(response).await;
        assert!(page.starts_with("<!doctype html>\n<html lang=\"pl\">\n<head>\n"));
        assert!(page.contains("<title>Zalogowano do &lt;Stellarr&gt;</title>"));
        assert!(page.ends_with(
            "<body><main><h1>Zalogowano do &lt;Stellarr&gt;</h1><p>Możesz zamknąć tę kartę</p></main></body>\n</html>"
        ));
        assert!(!page.ends_with('\n'));
    }

    #[tokio::test]
    async fn nothing_a_person_or_a_request_wrote_can_leave_the_attribute_it_is_in() {
        let hostile = "\"><script>alert(1)</script>";
        let response = confirmation(&Confirmation {
            language: "en",
            heading: hostile,
            body: hostile,
            button: hostile,
            action: hostile,
            token: hostile,
        });
        let page = text(response).await;
        assert!(!page.contains("<script"), "{page}");
        assert_eq!(page.matches("&quot;&gt;&lt;script&gt;").count(), 6);
        assert!(page.contains("<form method=\"post\" action=\"&quot;&gt;"));
        assert!(page.contains("name=\"token\" value=\"&quot;&gt;"));
    }
}
