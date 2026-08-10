use crate::tool_error::ToolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidArguments,
    CapacityExhausted,
    Timeout,
    Unauthorized,
    NotAllowed,
    NotFound,
    RequestRejected,
    UpstreamUnavailable,
    ResponseTooLarge,
    InvalidResponse,
}

impl Error {
    pub fn into_tool_error(self, action_name: &str) -> ToolError {
        let (code, message, retryable) = match self {
            Self::InvalidArguments => (
                "invalid_arguments",
                format!("The {action_name} arguments are invalid."),
                false,
            ),
            Self::CapacityExhausted => (
                "capacity_exhausted",
                "Home Assistant query capacity is currently exhausted.".to_owned(),
                true,
            ),
            Self::Timeout => (
                "timeout",
                "The Home Assistant query timed out.".to_owned(),
                true,
            ),
            Self::Unauthorized => (
                "home_assistant_unauthorized",
                "Home Assistant rejected the service credentials.".to_owned(),
                false,
            ),
            Self::NotAllowed => (
                "not_allowed",
                "One or more entities are not explicitly exposed to Assist.".to_owned(),
                false,
            ),
            Self::NotFound => (
                "entity_not_found",
                "One or more requested entities were not found.".to_owned(),
                false,
            ),
            Self::RequestRejected => (
                "request_rejected",
                "Home Assistant rejected the query.".to_owned(),
                false,
            ),
            Self::UpstreamUnavailable => (
                "upstream_unavailable",
                "Home Assistant is currently unavailable.".to_owned(),
                true,
            ),
            Self::ResponseTooLarge => (
                "response_too_large",
                "The Home Assistant response exceeded the safe limit.".to_owned(),
                false,
            ),
            Self::InvalidResponse => (
                "invalid_response",
                "Home Assistant returned an invalid response.".to_owned(),
                false,
            ),
        };
        ToolError::new(code, message, retryable)
    }
}
