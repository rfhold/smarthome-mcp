use reqwest::{Client, Method, Request, Response, StatusCode};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware, Result};
use reqwest_tracing::{ReqwestOtelSpanBackend, TracingMiddleware};
use tracing::{Span, field};

pub(crate) fn with_tracing(client: Client) -> ClientWithMiddleware {
    ClientBuilder::new(client)
        .with(TracingMiddleware::<ApplicationSpanBackend>::new())
        .build()
}

#[derive(Clone)]
pub(crate) struct RequestSpanExtension(Span);

pub(crate) struct RequestSpan {
    span: Span,
    finished: bool,
}

impl RequestSpan {
    pub(crate) fn new(method: &Method) -> Self {
        Self {
            span: tracing::info_span!(
                target: "smarthome_mcp::http_client",
                "http.client.request",
                otel.kind = "client",
                otel.status_code = field::Empty,
                http.request.method = %method,
                http.response.status_code = field::Empty,
                outcome = field::Empty,
            ),
            finished: false,
        }
    }

    pub(crate) fn extension(&self) -> RequestSpanExtension {
        RequestSpanExtension(self.span.clone())
    }

    pub(crate) fn span(&self) -> Span {
        self.span.clone()
    }

    pub(crate) fn record_status(&self, status: StatusCode) {
        self.span
            .record("http.response.status_code", status.as_u16());
    }

    pub(crate) fn success(&mut self) {
        self.finish("success", false);
    }

    pub(crate) fn http_error(&mut self, status: StatusCode) {
        self.record_status(status);
        self.finish(
            "http_error",
            status.is_client_error() || status.is_server_error(),
        );
    }

    pub(crate) fn transport_error(&mut self) {
        self.finish("transport_error", true);
    }

    pub(crate) fn response_error(&mut self) {
        self.finish("response_error", true);
    }

    fn finish(&mut self, outcome: &'static str, error: bool) {
        if self.finished {
            return;
        }
        self.span.record("outcome", outcome);
        if error {
            self.span.record("otel.status_code", "ERROR");
        }
        self.finished = true;
        self.span = Span::none();
    }
}

impl Drop for RequestSpan {
    fn drop(&mut self) {
        if !self.finished {
            self.span.record("outcome", "cancelled");
            self.span.record("otel.status_code", "ERROR");
        }
    }
}

struct ApplicationSpanBackend;

impl ReqwestOtelSpanBackend for ApplicationSpanBackend {
    fn on_request_start(_request: &Request, extensions: &mut http::Extensions) -> Span {
        extensions
            .get::<RequestSpanExtension>()
            .map_or_else(Span::none, |request_span| request_span.0.clone())
    }

    fn on_request_end(
        _span: &Span,
        _result: &Result<Response>,
        _extensions: &mut http::Extensions,
    ) {
    }
}
