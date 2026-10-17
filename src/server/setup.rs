//! The HTML setup pages the Alpaca Management API defines, served for `GET`
//! and `POST`: `/setup` for the whole server, answered by the [`ServerSetup`]
//! handler a [`Server`](crate::Server) carries, and
//! `/setup/v1/{device_type}/{device_number}/setup` for one device, answered by
//! that device's [`Device::setup_request`](crate::api::Device::setup_request).

use crate::api::{DevicePath, DeviceType};
use axum::Form;
use axum::extract::{FromRequest, Query, Request};
use axum::response::{IntoResponse, Response};
use futures::future::BoxFuture;
use http::header::{ALLOW, CONTENT_TYPE, LOCATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use std::fmt::Debug;

/// A request for one of the setup pages.
///
/// The router builds one for every `GET` and `POST` on a setup path. The query
/// string is parsed for both methods; the body is parsed for a `POST`, which
/// must carry a form (`application/x-www-form-urlencoded`) — any other body is
/// refused with `415 Unsupported Media Type` before the handler runs. Pairs keep
/// the order they were sent in, repeated names included, so a group of
/// checkboxes under one name arrives whole.
#[derive(Debug, Clone)]
pub struct SetupRequest {
    /// `GET` or `POST`; the router answers every other method with `405`.
    pub method: Method,
    /// The request's headers, for an origin check or content negotiation.
    pub headers: HeaderMap,
    /// The query string's pairs, in order.
    pub query: Vec<(String, String)>,
    /// The form body's pairs, in order; empty for a `GET`.
    pub form: Vec<(String, String)>,
}

impl<S: Send + Sync> FromRequest<S> for SetupRequest {
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let method = req.method().clone();
        let headers = req.headers().clone();
        let query = Query::<Vec<(String, String)>>::try_from_uri(req.uri())
            .map_err(IntoResponse::into_response)?
            .0;
        let form = if method == Method::POST {
            Form::<Vec<(String, String)>>::from_request(req, state)
                .await
                .map_err(IntoResponse::into_response)?
                .0
        } else {
            Vec::new()
        };
        Ok(Self {
            method,
            headers,
            query,
            form,
        })
    }
}

/// The answer to a [`SetupRequest`]: a status, headers and a body.
#[derive(Debug, Clone)]
pub struct SetupResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl SetupResponse {
    /// An HTML page, answered `200 OK`.
    pub fn html(html: impl Into<String>) -> Self {
        Self::with_content_type(StatusCode::OK, "text/html; charset=utf-8", html)
    }

    /// Plain text under the given status: an error message, most often.
    pub fn text(status: StatusCode, text: impl Into<String>) -> Self {
        Self::with_content_type(status, "text/plain; charset=utf-8", text)
    }

    /// A `303 See Other` to `location`: the answer to a `POST` that was acted
    /// on, so that the browser's next `GET` shows the result and a refresh
    /// does not resubmit the form.
    ///
    /// A location that is not a valid header value (a control character, a
    /// byte outside ASCII) cannot be sent, so it is answered as a `500` that
    /// says so.
    pub fn see_other(location: &str) -> Self {
        HeaderValue::from_str(location).map_or_else(
            |_| {
                Self::text(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("redirect location {location:?} is not a valid header value"),
                )
            },
            |location| {
                Self::text(StatusCode::SEE_OTHER, String::new()).with_header(LOCATION, location)
            },
        )
    }

    /// `405 Method Not Allowed`, naming `GET` as the method the page takes:
    /// the default answer to a `POST` on a page that has no form.
    pub fn method_not_allowed() -> Self {
        Self::text(
            StatusCode::METHOD_NOT_ALLOWED,
            "This setup page takes GET only",
        )
        .with_header(ALLOW, HeaderValue::from_static("GET"))
    }

    /// The same response under another status: a form re-rendered with its
    /// errors under `422 Unprocessable Content`, for example.
    #[must_use]
    pub const fn with_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// The same response with a header added.
    #[must_use]
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        _ = self.headers.append(name, value);
        self
    }

    /// The status.
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// The headers.
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The body.
    pub fn body(&self) -> &str {
        &self.body
    }

    fn with_content_type(
        status: StatusCode,
        content_type: &'static str,
        body: impl Into<String>,
    ) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.into(),
        }
        .with_header(CONTENT_TYPE, HeaderValue::from_static(content_type))
    }
}

impl IntoResponse for SetupResponse {
    fn into_response(self) -> Response {
        (self.status, self.headers, self.body).into_response()
    }
}

/// The handler behind the whole-server setup page, `/setup`.
///
/// The Management API has this page "enable the user to set cross cutting
/// information that applies to the whole device such as the operating IP
/// address and port number". A [`Server`](crate::Server) with no handler
/// serves a list of its devices, each linked to its own page, and answers a
/// `POST` with `405 Method Not Allowed`.
pub trait ServerSetup: Debug + Send + Sync {
    /// Answer a `GET` or `POST` on `/setup`.
    fn setup_request(&self, request: SetupRequest) -> BoxFuture<'_, SetupResponse>;
}

/// The device type a path segment names, if the crate serves that type.
pub(super) fn device_type_from_path(segment: &str) -> Option<DeviceType> {
    DevicePath::deserialize(
        serde::de::value::StrDeserializer::<serde::de::value::Error>::new(segment),
    )
    .ok()
}

/// The `403 Forbidden` the Management API gives a setup path that names a
/// device the server does not have: "the provided path specifies an
/// unsupported device or method, return an error message to display in the
/// browser".
pub(super) fn unsupported_device(device_type: &str, device_number: &str) -> SetupResponse {
    SetupResponse::html(format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"UTF-8\"><title>Unknown device</title></head>\
         <body><p>This server has no {} device numbered {}.</p></body></html>\n",
        escape_html(device_type),
        escape_html(device_number),
    ))
    .with_status(StatusCode::FORBIDDEN)
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

#[cfg(all(test, feature = "safety_monitor"))]
mod tests {
    use super::*;
    use crate::api::{Device, SafetyMonitor, ServerInfo};
    use crate::{ASCOMResult, Server};
    use axum::Router;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use std::sync::{Arc, Mutex};
    use tower_service::Service;

    /// A safety monitor with the crate's default setup page.
    #[derive(Debug)]
    struct PlainMonitor;

    #[async_trait::async_trait]
    impl Device for PlainMonitor {
        fn static_name(&self) -> &'static str {
            "Plain"
        }

        fn unique_id(&self) -> &'static str {
            "plain-monitor"
        }

        async fn connected(&self) -> ASCOMResult<bool> {
            Ok(true)
        }

        async fn set_connected(&self, _connected: bool) -> ASCOMResult<()> {
            Ok(())
        }

        async fn description(&self) -> ASCOMResult<String> {
            Ok("Plain monitor".to_owned())
        }

        async fn driver_info(&self) -> ASCOMResult<String> {
            Ok("Plain monitor".to_owned())
        }

        async fn driver_version(&self) -> ASCOMResult<String> {
            Ok("1.0".to_owned())
        }
    }

    #[async_trait::async_trait]
    impl SafetyMonitor for PlainMonitor {
        async fn is_safe(&self) -> ASCOMResult<bool> {
            Ok(true)
        }
    }

    /// A safety monitor with a form on its page: it records the requests it
    /// sees, answers a `GET` with the form and a `POST` with the form again
    /// under `422`.
    #[derive(Debug)]
    struct FormMonitor {
        seen: Arc<Mutex<Vec<SetupRequest>>>,
    }

    #[async_trait::async_trait]
    impl Device for FormMonitor {
        fn static_name(&self) -> &'static str {
            "Form"
        }

        fn unique_id(&self) -> &'static str {
            "form-monitor"
        }

        fn setup_request(&self, request: SetupRequest) -> BoxFuture<'_, SetupResponse> {
            Box::pin(async move {
                let method = request.method.clone();
                self.seen
                    .lock()
                    .expect("the seen lock is never poisoned")
                    .push(request);
                let page = SetupResponse::html("<form method=post></form>");
                if method == Method::POST {
                    page.with_status(StatusCode::UNPROCESSABLE_ENTITY)
                } else {
                    page
                }
            })
        }

        async fn connected(&self) -> ASCOMResult<bool> {
            Ok(true)
        }

        async fn set_connected(&self, _connected: bool) -> ASCOMResult<()> {
            Ok(())
        }

        async fn description(&self) -> ASCOMResult<String> {
            Ok("Form monitor".to_owned())
        }

        async fn driver_info(&self) -> ASCOMResult<String> {
            Ok("Form monitor".to_owned())
        }

        async fn driver_version(&self) -> ASCOMResult<String> {
            Ok("1.0".to_owned())
        }
    }

    #[async_trait::async_trait]
    impl SafetyMonitor for FormMonitor {
        async fn is_safe(&self) -> ASCOMResult<bool> {
            Ok(true)
        }
    }

    /// A server page that records the requests it sees and answers a `POST`
    /// with a redirect back to itself.
    #[derive(Debug)]
    struct RecordingServerSetup {
        seen: Arc<Mutex<Vec<SetupRequest>>>,
    }

    impl ServerSetup for RecordingServerSetup {
        fn setup_request(&self, request: SetupRequest) -> BoxFuture<'_, SetupResponse> {
            Box::pin(async move {
                let method = request.method.clone();
                self.seen
                    .lock()
                    .expect("the seen lock is never poisoned")
                    .push(request);
                if method == Method::POST {
                    SetupResponse::see_other("/setup?saved")
                } else {
                    SetupResponse::html("<h1>Server settings</h1>")
                }
            })
        }
    }

    fn server_info() -> ServerInfo {
        ServerInfo {
            server_name: "test".into(),
            manufacturer: "test".into(),
            manufacturer_version: "test".into(),
            location: "test".into(),
        }
    }

    /// A router serving one plain monitor as device 0.
    fn plain_router() -> Router {
        let mut server = Server::new(server_info());
        server.devices.register(PlainMonitor);
        server.into_router_inner()
    }

    /// One request through the router.
    async fn send(
        router: &Router,
        method: &str,
        uri: &str,
        content_type: Option<&str>,
        body: &str,
    ) -> (StatusCode, HeaderMap, String) {
        let mut service = router.clone();
        futures::future::poll_fn(|cx| {
            <Router as Service<http::Request<Body>>>::poll_ready(&mut service, cx)
        })
        .await
        .expect("the router is always ready");
        let mut request = http::Request::builder().method(method).uri(uri);
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let response = service
            .call(
                request
                    .body(Body::from(body.to_owned()))
                    .expect("the request is well-formed"),
            )
            .await
            .expect("the router is infallible");
        let (parts, body) = response.into_parts();
        let body = body
            .collect()
            .await
            .expect("the response body is readable")
            .to_bytes();
        (
            parts.status,
            parts.headers,
            String::from_utf8(body.to_vec()).expect("the response body is UTF-8"),
        )
    }

    async fn get(router: &Router, uri: &str) -> (StatusCode, HeaderMap, String) {
        send(router, "GET", uri, None, "").await
    }

    async fn post_form(router: &Router, uri: &str, form: &str) -> (StatusCode, HeaderMap, String) {
        send(
            router,
            "POST",
            uri,
            Some("application/x-www-form-urlencoded"),
            form,
        )
        .await
    }

    fn header<'headers>(headers: &'headers HeaderMap, name: &HeaderName) -> &'headers str {
        headers
            .get(name)
            .unwrap_or_else(|| panic!("no {name} header"))
            .to_str()
            .expect("the header is ASCII")
    }

    #[tokio::test]
    async fn the_server_page_links_each_device_to_its_setup_path() {
        let (status, headers, body) = get(&plain_router(), "/setup").await;
        assert_eq!(status, StatusCode::OK);
        assert!(header(&headers, &CONTENT_TYPE).starts_with("text/html"));
        // The page's own markup leaves attribute values unquoted.
        assert!(
            body.contains("href=/setup/v1/safetymonitor/0/setup>"),
            "the page links the device's own page: {body}"
        );
        assert!(
            !body.contains("/api/v1/"),
            "no link under the API path remains: {body}"
        );
    }

    #[tokio::test]
    async fn a_post_to_the_server_page_without_a_handler_is_not_allowed() {
        let (status, headers, _) = post_form(&plain_router(), "/setup", "a=1").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(header(&headers, &ALLOW), "GET");
    }

    #[tokio::test]
    async fn a_device_without_a_page_of_its_own_answers_the_default() {
        let (status, headers, body) = get(&plain_router(), "/setup/v1/safetymonitor/0/setup").await;
        assert_eq!(status, StatusCode::OK);
        assert!(header(&headers, &CONTENT_TYPE).starts_with("text/html"));
        assert!(
            body.contains("does not support configuration"),
            "the default page says the device is not configurable: {body}"
        );
    }

    #[tokio::test]
    async fn a_post_to_a_device_without_a_form_is_not_allowed() {
        let (status, headers, _) =
            post_form(&plain_router(), "/setup/v1/safetymonitor/0/setup", "a=1").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(header(&headers, &ALLOW), "GET");
    }

    #[tokio::test]
    async fn an_unknown_device_number_is_forbidden() {
        let (status, headers, body) = get(&plain_router(), "/setup/v1/safetymonitor/1/setup").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(header(&headers, &CONTENT_TYPE).starts_with("text/html"));
        assert!(
            body.contains("no safetymonitor device numbered 1"),
            "the message names the device: {body}"
        );
    }

    #[tokio::test]
    async fn an_unknown_device_type_is_forbidden() {
        let (status, _, body) = get(&plain_router(), "/setup/v1/thermostat/0/setup").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("no thermostat device numbered 0"), "{body}");
    }

    #[tokio::test]
    async fn a_device_number_that_is_not_a_number_is_forbidden() {
        let (status, _, body) = get(&plain_router(), "/setup/v1/safetymonitor/first/setup").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("numbered first"), "{body}");
    }

    #[tokio::test]
    async fn the_message_for_an_unknown_device_escapes_the_path() {
        let (status, _, body) = get(&plain_router(), "/setup/v1/%3Cscript%3E/0/setup").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("&lt;script&gt;"), "{body}");
        assert!(!body.contains("<script>"), "{body}");
    }

    #[tokio::test]
    async fn a_method_other_than_get_and_post_is_not_allowed() {
        let router = plain_router();
        let (status, _, _) = send(&router, "PUT", "/setup", None, "").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        let (status, _, _) = send(
            &router,
            "DELETE",
            "/setup/v1/safetymonitor/0/setup",
            None,
            "",
        )
        .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn the_server_handler_sees_the_query_and_the_form() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server::new(server_info());
        server.setup = Some(Arc::new(RecordingServerSetup {
            seen: Arc::clone(&seen),
        }));
        let router = server.into_router_inner();

        let (status, _, body) = get(&router, "/setup?tab=network").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "<h1>Server settings</h1>");

        let (status, headers, _) =
            post_form(&router, "/setup?tab=network", "port=11119&dns=a&dns=b").await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(header(&headers, &LOCATION), "/setup?saved");

        let seen = seen.lock().expect("the seen lock is never poisoned");
        assert_eq!(seen.len(), 2, "both requests reached the handler");
        assert_eq!(seen[0].method, Method::GET);
        assert_eq!(seen[0].query, [("tab".to_owned(), "network".to_owned())]);
        assert!(seen[0].form.is_empty(), "a GET carries no form");
        assert_eq!(seen[1].method, Method::POST);
        assert_eq!(seen[1].query, [("tab".to_owned(), "network".to_owned())]);
        assert_eq!(
            seen[1].form,
            [
                ("port".to_owned(), "11119".to_owned()),
                ("dns".to_owned(), "a".to_owned()),
                ("dns".to_owned(), "b".to_owned()),
            ],
            "the form keeps its order and its repeated names"
        );
    }

    #[tokio::test]
    async fn a_post_whose_body_is_not_a_form_is_refused_before_the_handler() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server::new(server_info());
        server.setup = Some(Arc::new(RecordingServerSetup {
            seen: Arc::clone(&seen),
        }));
        let router = server.into_router_inner();

        let (status, _, _) = send(&router, "POST", "/setup", Some("application/json"), "{}").await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(
            seen.lock()
                .expect("the seen lock is never poisoned")
                .is_empty(),
            "the handler never saw the request"
        );
    }

    #[tokio::test]
    async fn a_device_hook_sees_the_form_and_sets_its_own_status() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server::new(server_info());
        server.devices.register(FormMonitor {
            seen: Arc::clone(&seen),
        });
        let router = server.into_router_inner();

        let (status, _, body) = get(&router, "/setup/v1/safetymonitor/0/setup").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "<form method=post></form>");

        let (status, headers, body) =
            post_form(&router, "/setup/v1/safetymonitor/0/setup", "name=Roof").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(header(&headers, &CONTENT_TYPE).starts_with("text/html"));
        assert_eq!(body, "<form method=post></form>");

        let seen = seen.lock().expect("the seen lock is never poisoned");
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].form, [("name".to_owned(), "Roof".to_owned())]);
    }

    #[test]
    fn a_redirect_location_that_is_not_a_header_value_is_an_error() {
        let response = SetupResponse::see_other("/setup\r\nSet-Cookie: x");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.headers().get(LOCATION).is_none());
        assert!(response.body().contains("not a valid header value"));
    }

    #[test]
    fn a_response_carries_the_headers_it_was_given() {
        let response = SetupResponse::html("<p>")
            .with_status(StatusCode::CREATED)
            .with_header(
                HeaderName::from_static("x-page"),
                HeaderValue::from_static("device"),
            );
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            header(response.headers(), &CONTENT_TYPE),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            header(response.headers(), &HeaderName::from_static("x-page")),
            "device"
        );
        assert_eq!(response.body(), "<p>");
    }

    #[test]
    fn html_special_characters_are_escaped() {
        assert_eq!(escape_html("a<b>&\"c'"), "a&lt;b&gt;&amp;&quot;c&#39;");
    }
}
