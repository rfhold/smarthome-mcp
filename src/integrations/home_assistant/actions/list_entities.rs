use schemars::JsonSchema;
use serde::Deserialize;

use super::valid_name;

const DEFAULT_LIMIT: u16 = 50;
const MAX_LIMIT: u16 = 100;
const MAX_QUERY_BYTES: usize = 128;
const MAX_DOMAINS: usize = 20;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEntitiesInput {
    /// Case-insensitive substring matched against entity IDs and friendly names.
    pub query: Option<String>,
    /// Optional Home Assistant domains, such as `sensor` or `light`.
    pub domains: Option<Vec<String>>,
    /// Maximum entities to return. Defaults to 50 and cannot exceed 100.
    pub limit: Option<u16>,
}

pub(crate) struct EntitiesQuery {
    pub(crate) query: Option<String>,
    pub(crate) domains: Vec<String>,
    pub(crate) limit: usize,
}

impl ListEntitiesInput {
    pub(crate) fn validate(self) -> Result<EntitiesQuery, ()> {
        let query = self
            .query
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        if query
            .as_ref()
            .is_some_and(|value| value.len() > MAX_QUERY_BYTES)
        {
            return Err(());
        }
        let mut domains = self.domains.unwrap_or_default();
        if domains.len() > MAX_DOMAINS || domains.iter().any(|domain| !valid_name(domain)) {
            return Err(());
        }
        domains.sort();
        domains.dedup();
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(());
        }
        Ok(EntitiesQuery {
            query,
            domains,
            limit: usize::from(limit),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_bounded_defaults() {
        let query = ListEntitiesInput {
            query: None,
            domains: None,
            limit: None,
        }
        .validate()
        .unwrap();
        assert_eq!(query.limit, 50);
    }

    #[test]
    fn rejects_invalid_filters() {
        assert!(
            ListEntitiesInput {
                query: None,
                domains: Some(vec!["Bad-Domain".to_owned()]),
                limit: None,
            }
            .validate()
            .is_err()
        );
        assert!(
            ListEntitiesInput {
                query: None,
                domains: None,
                limit: Some(101),
            }
            .validate()
            .is_err()
        );
    }
}
