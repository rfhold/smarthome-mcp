use crate::{
    config::IntegrationsConfig,
    integrations::home_assistant::{ComponentDeployer, HomeAssistantClient},
};

#[derive(Clone)]
pub struct Services {
    pub(crate) home_assistant: HomeAssistantClient,
    pub(crate) component_deployer: ComponentDeployer,
}

impl Services {
    pub fn production(config: &IntegrationsConfig) -> Result<Self, String> {
        Ok(Self {
            home_assistant: HomeAssistantClient::production(
                config.home_assistant.origin.clone(),
                config.home_assistant.token.clone(),
            )?,
            component_deployer: ComponentDeployer::production(&config.home_assistant.ssh)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn new(home_assistant: HomeAssistantClient) -> Self {
        Self {
            home_assistant,
            component_deployer: ComponentDeployer::unavailable(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_component_deployer(
        home_assistant: HomeAssistantClient,
        component_deployer: ComponentDeployer,
    ) -> Self {
        Self {
            home_assistant,
            component_deployer,
        }
    }
}
