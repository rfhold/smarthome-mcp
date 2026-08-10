use crate::{config::IntegrationsConfig, integrations::home_assistant::HomeAssistantClient};

#[derive(Clone)]
pub struct Services {
    pub(crate) home_assistant: HomeAssistantClient,
}

impl Services {
    pub fn production(config: &IntegrationsConfig) -> Result<Self, String> {
        Ok(Self {
            home_assistant: HomeAssistantClient::production(
                config.home_assistant.origin.clone(),
                config.home_assistant.token.clone(),
            )?,
        })
    }

    #[cfg(test)]
    pub(crate) fn new(home_assistant: HomeAssistantClient) -> Self {
        Self { home_assistant }
    }
}
