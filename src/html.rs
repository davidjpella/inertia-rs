//! Initial HTML response helpers.

use crate::ssr::SsrResponse;
use serde::Serialize;

/// Escapes serialized JSON for embedding inside an HTML `<script>` element.
///
/// `<`, `>`, `&`, U+2028, and U+2029 become JSON unicode escapes, so the
/// output can never close the script element early and still parses as the
/// same JSON. HTML entities are never used: browsers do not decode them inside
/// script elements.
pub fn escape_json_for_html_script(json: &str) -> String {
    let mut escaped = String::with_capacity(json.len());

    for c in json.chars() {
        match c {
            '<' => escaped.push_str("\\u003C"),
            '>' => escaped.push_str("\\u003E"),
            '&' => escaped.push_str("\\u0026"),
            '\u{2028}' => escaped.push_str("\\u2028"),
            '\u{2029}' => escaped.push_str("\\u2029"),
            _ => escaped.push(c),
        }
    }

    escaped
}

fn escape_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }

    escaped
}

/// Context passed to framework HTML renderers for first-page loads.
///
/// Templates usually print [`head`](Self::head) inside `<head>` and
/// [`body`](Self::body) where the app mounts. Without SSR, `body` renders the
/// page-object script and the empty root element; with SSR it renders the
/// server-rendered markup, which already contains both.
///
/// ```rust
/// use inertia_rs::HtmlResponseContext;
///
/// let context = HtmlResponseContext::new(r#"{"component":"Home"}"#);
/// assert_eq!(
///     context.body(),
///     r#"<script data-page="app" type="application/json">{"component":"Home"}</script><div id="app"></div>"#
/// );
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HtmlResponseContext {
    data_page: String,
    root_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ssr: Option<SsrResponse>,
}

impl HtmlResponseContext {
    /// Creates a context from a serialized page object string.
    ///
    /// Framework integrations construct this with script-safe JSON. If you
    /// create it manually, escape the JSON with
    /// [`escape_json_for_html_script`].
    pub fn new<D: Into<String>>(data_page: D) -> Self {
        Self {
            data_page: data_page.into(),
            root_id: "app".to_owned(),
            ssr: None,
        }
    }

    /// Serializes `page` into a script-safe context.
    pub fn from_page<T: Serialize>(page: &T) -> Result<Self, serde_json::Error> {
        serde_json::to_string(page)
            .map(|json| escape_json_for_html_script(&json))
            .map(Self::new)
    }

    /// Sets the root element id (default `app`). It must match the client's
    /// `createInertiaApp({ id })`.
    pub fn with_root_id<I: Into<String>>(mut self, root_id: I) -> Self {
        self.root_id = root_id.into();
        self
    }

    /// Attaches a server-side rendering result.
    pub fn with_ssr(mut self, ssr: SsrResponse) -> Self {
        self.ssr = Some(ssr);
        self
    }

    /// Returns the script-safe JSON page object.
    pub fn data_page(&self) -> &str {
        &self.data_page
    }

    /// Returns the root element id.
    pub fn root_id(&self) -> &str {
        &self.root_id
    }

    /// Returns `true` when the page was server-side rendered.
    pub fn is_server_rendered(&self) -> bool {
        self.ssr.is_some()
    }

    /// Returns the SSR result, if any.
    pub fn ssr(&self) -> Option<&SsrResponse> {
        self.ssr.as_ref()
    }

    /// Returns the SSR head elements joined by newlines, or an empty string.
    ///
    /// Print fallback `<title>`/`<meta>` tags only when this is empty, or mark
    /// them with `data-inertia` so the client `<Head>` component adopts them.
    pub fn head(&self) -> String {
        self.ssr
            .as_ref()
            .map(|ssr| ssr.head().join("\n"))
            .unwrap_or_default()
    }

    /// Returns the markup for the app root.
    ///
    /// SSR markup is returned unchanged: the SSR server escapes the embedded
    /// page object itself.
    pub fn body(&self) -> String {
        match &self.ssr {
            Some(ssr) => ssr.body().to_owned(),
            None => format!(
                r#"<script data-page="{id}" type="application/json">{page}</script><div id="{id}"></div>"#,
                id = escape_attribute(&self.root_id),
                page = self.data_page,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_script_breaking_sequences() {
        let escaped = escape_json_for_html_script(
            r#"{"t":"</script><script>alert(1)</script>&<!--","s":"  "}"#,
        );

        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('>'));
        assert!(!escaped.contains('&'));
        let parsed: serde_json::Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(parsed["t"], "</script><script>alert(1)</script>&<!--");
    }

    #[test]
    fn root_id_is_attribute_escaped() {
        let body = HtmlResponseContext::new("{}").with_root_id("a\"b").body();

        assert!(body.contains(r#"data-page="a&quot;b""#));
        assert!(body.contains(r#"id="a&quot;b""#));
    }

    #[test]
    fn ssr_body_and_head_replace_client_markup() {
        let context = HtmlResponseContext::new("{}").with_ssr(SsrResponse::new(
            vec!["<title>A</title>".into(), "<meta name=x>".into()],
            "<div data-server-rendered=\"true\" id=\"app\">A</div>",
        ));

        assert_eq!(context.head(), "<title>A</title>\n<meta name=x>");
        assert_eq!(
            context.body(),
            "<div data-server-rendered=\"true\" id=\"app\">A</div>"
        );
        assert!(context.is_server_rendered());
    }
}
