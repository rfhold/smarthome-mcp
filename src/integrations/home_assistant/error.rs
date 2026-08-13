use crate::tool_error::ToolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidArguments,
    CapacityExhausted,
    Timeout,
    Unauthorized,
    NotAllowed,
    NotMatterDevice,
    NotFound,
    ConfigNotFound,
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
                "Home Assistant operation capacity is currently exhausted.".to_owned(),
                true,
            ),
            Self::Timeout => (
                "timeout",
                "The Home Assistant operation timed out.".to_owned(),
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
            Self::NotMatterDevice => (
                "not_matter_device",
                "The requested device is not a Matter device.".to_owned(),
                false,
            ),
            Self::NotFound => (
                "entity_not_found",
                "One or more requested entities were not found.".to_owned(),
                false,
            ),
            Self::ConfigNotFound => (
                "config_not_found",
                "The requested Home Assistant configuration was not found.".to_owned(),
                false,
            ),
            Self::RequestRejected => (
                "request_rejected",
                "Home Assistant rejected the operation.".to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_safe_errors_are_operation_neutral() {
        for (error, code, message, retryable) in [
            (
                Error::CapacityExhausted,
                "capacity_exhausted",
                "Home Assistant operation capacity is currently exhausted.",
                true,
            ),
            (
                Error::Timeout,
                "timeout",
                "The Home Assistant operation timed out.",
                true,
            ),
            (
                Error::RequestRejected,
                "request_rejected",
                "Home Assistant rejected the operation.",
                false,
            ),
        ] {
            let value = error
                .into_tool_error("control entity")
                .into_mcp_result()
                .raw;
            assert_eq!(value["structuredContent"]["error"]["code"], code);
            assert_eq!(value["structuredContent"]["error"]["message"], message);
            assert_eq!(value["structuredContent"]["error"]["retryable"], retryable);
            assert!(!serde_json::to_string(&value).unwrap().contains("query"));
        }
    }

    #[test]
    fn config_not_found_is_specific_and_private() {
        let value = Error::ConfigNotFound
            .into_tool_error("get automation")
            .into_mcp_result()
            .raw;
        assert_eq!(
            value["structuredContent"]["error"]["code"],
            "config_not_found"
        );
        assert_eq!(
            value["structuredContent"]["error"]["message"],
            "The requested Home Assistant configuration was not found."
        );
    }
}
