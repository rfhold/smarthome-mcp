use super::*;
use crate::integrations::home_assistant::{Error, HomeAssistantClient, actions::*};
use sha2::{Digest, Sha256};

const ITEM_LIMIT: usize = 256 * 1024;

fn closed(value: &serde_json::Value, fields: &[&str]) -> ServerResult<()> {
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != fields.len() || !fields.iter().all(|field| object.contains_key(*field)) {
        return Err(invalid());
    }
    Ok(())
}

fn invalid() -> ServerError {
    ServerError::invalid_params("invalid authoring arguments")
}

fn text<'a>(value: &'a serde_json::Value, field: &str, nonempty: bool) -> ServerResult<&'a str> {
    value[field]
        .as_str()
        .filter(|s| (!nonempty || !s.is_empty()) && s.len() <= ITEM_LIMIT && !s.contains('\0'))
        .ok_or_else(invalid)
}

pub(super) fn target(uri: &str) -> ServerResult<(&'static str, String)> {
    let path = uri.strip_prefix("smarthome://").ok_or_else(invalid)?;
    let (catalog, encoded) = path.split_once('/').ok_or_else(invalid)?;
    let kind = match catalog {
        "scenes" => "scene",
        "automations" => "automation",
        "blueprints" => "blueprint",
        _ => return Err(invalid()),
    };
    let key = decode_item_id(encoded)?;
    if item_uri(catalog, &key) != uri {
        return Err(invalid());
    }
    validate_key(kind, &key)?;
    Ok((kind, key))
}

fn validate_key(kind: &str, key: &str) -> ServerResult<()> {
    if kind == "blueprint" {
        crate::integrations::home_assistant::actions::BlueprintGetInput { path: key.into() }
            .validate()
            .map_err(|_| invalid())?;
    } else if !crate::integrations::home_assistant::actions::valid_config_key(key) {
        return Err(invalid());
    }
    Ok(())
}

fn revision(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn payload(name: &str, args: &serde_json::Value) -> ServerResult<serde_json::Value> {
    if serde_json::to_vec(args).map_err(|_| invalid())?.len() > MAX_TEXT_RESOURCE_BYTES {
        return Err(invalid());
    }
    if name == "create" {
        closed(args, &["action", "input"])?;
        let kind = match args["action"].as_str() {
            Some("scene.create") => "scene",
            Some("automation.create") => "automation",
            Some("blueprint.create") => "blueprint",
            _ => return Err(invalid()),
        };
        let field = if kind == "blueprint" {
            "path"
        } else {
            "config_key"
        };
        closed(&args["input"], &[field, "text"])?;
        let key = text(&args["input"], field, true)?;
        validate_key(kind, key)?;
        let native = text(&args["input"], "text", true)?;
        return Ok(json!({"kind":kind,"key":key,"text":native}));
    }
    closed(args, &["uri", "expected_revision", "edits"])?;
    let (kind, key) = target(text(args, "uri", true)?)?;
    if !revision(text(args, "expected_revision", true)?) {
        return Err(invalid());
    }
    let edits = args["edits"]
        .as_array()
        .filter(|e| (1..=256).contains(&e.len()))
        .ok_or_else(invalid)?;
    for edit in edits {
        match edit["operation"].as_str() {
            Some("replace") => {
                closed(edit, &["operation", "old_text", "new_text"])?;
                text(edit, "old_text", true)?;
                text(edit, "new_text", false)?;
            }
            Some("insert") => {
                match edit["placement"].as_str() {
                    Some("start" | "end") => closed(edit, &["operation", "text", "placement"])?,
                    Some("before" | "after") => {
                        closed(edit, &["operation", "text", "placement", "anchor"])?;
                        text(edit, "anchor", true)?;
                    }
                    _ => return Err(invalid()),
                }
                text(edit, "text", false)?;
            }
            _ => return Err(invalid()),
        }
    }
    Ok(json!({"kind":kind,"key":key,"expected_revision":args["expected_revision"],"edits":edits}))
}

fn digest(text: &str) -> String {
    format!(
        "sha256:{}",
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn unique(text: &str, needle: &str) -> ServerResult<usize> {
    let index = text.find(needle).ok_or_else(invalid)?;
    let next = index + text[index..].chars().next().ok_or_else(invalid)?.len_utf8();
    if text[next..].contains(needle) {
        return Err(invalid());
    }
    Ok(index)
}

fn apply_edits(initial: &str, edits: &[serde_json::Value]) -> ServerResult<String> {
    if initial.len() > ITEM_LIMIT {
        return Err(invalid());
    }
    let mut result = initial.to_owned();
    for edit in edits {
        let (index, removed, inserted) = if edit["operation"] == "replace" {
            let old = text(edit, "old_text", true)?;
            (
                unique(&result, old)?,
                old.len(),
                text(edit, "new_text", false)?,
            )
        } else {
            let index = match edit["placement"].as_str().ok_or_else(invalid)? {
                "start" => 0,
                "end" => result.len(),
                placement @ ("before" | "after") => {
                    let anchor = text(edit, "anchor", true)?;
                    unique(&result, anchor)?
                        + if placement == "after" {
                            anchor.len()
                        } else {
                            0
                        }
                }
                _ => return Err(invalid()),
            };
            (index, 0, text(edit, "text", false)?)
        };
        if result.len() - removed + inserted.len() > ITEM_LIMIT
            || &result[index..index + removed] == inserted
        {
            return Err(invalid());
        }
        result.replace_range(index..index + removed, inserted);
    }
    if result == initial {
        return Err(invalid());
    }
    Ok(result)
}

enum Candidate {
    Config(ConfigUpsert),
    Blueprint {
        command: BlueprintSave,
        semantic: serde_yaml_ng::Value,
    },
}

fn blueprint_semantic(native: &str) -> ServerResult<serde_yaml_ng::Value> {
    if native.len() > ITEM_LIMIT {
        return Err(invalid());
    }
    let semantic: serde_yaml_ng::Value = serde_yaml_ng::from_str(native).map_err(|_| invalid())?;
    if !semantic.is_mapping() {
        return Err(invalid());
    }
    Ok(semantic)
}

fn candidate(kind: &str, key: &str, native: &str) -> ServerResult<Candidate> {
    if kind == "blueprint" {
        let command = BlueprintSaveInput {
            path: key.into(),
            yaml: native.into(),
        }
        .validate()
        .map_err(|_| invalid())?;
        return Ok(Candidate::Blueprint {
            command,
            semantic: blueprint_semantic(native)?,
        });
    }
    let config = serde_json::from_str(native).map_err(|_| invalid())?;
    ConfigUpsertInput {
        config_key: key.into(),
        config,
    }
    .validate()
    .map(Candidate::Config)
    .map_err(|_| invalid())
}

async fn read(client: &HomeAssistantClient, kind: &str, key: &str) -> Result<String, Error> {
    let value = match kind {
        "scene" => {
            client
                .get_scene(&ConfigGetQuery {
                    config_key: key.into(),
                })
                .await?
        }
        "automation" => {
            client
                .get_automation(&ConfigGetQuery {
                    config_key: key.into(),
                })
                .await?
        }
        _ => client.get_blueprint(&BlueprintPath(key.into())).await?,
    };
    let native = if kind == "blueprint" {
        value["yaml"]
            .as_str()
            .ok_or(Error::InvalidResponse)?
            .to_owned()
    } else {
        serde_json::to_string_pretty(&value["config"]).map_err(|_| Error::InvalidResponse)?
    };
    if native.len() > ITEM_LIMIT {
        return Err(Error::ResponseTooLarge);
    }
    Ok(native)
}

async fn preflight(
    client: &HomeAssistantClient,
    kind: &str,
    key: &str,
    create: bool,
) -> Result<String, Error> {
    if create && kind == "blueprint" {
        return if client.blueprint_absence(key).await? {
            Err(Error::ConfigNotFound)
        } else {
            Ok(String::new())
        };
    }
    read(client, kind, key).await
}

fn failure(code: &'static str) -> McpToolResult {
    crate::tool_error::ToolError::new(
        code,
        "Authoring request failed. Read before any further mutation.",
        false,
    )
    .into_mcp_result()
}

impl ResourceFirstMcp {
    pub(super) async fn authoring_resource(
        &self,
        uri: &str,
        context: ServerContext,
    ) -> ServerResult<mcp::McpResourceResult> {
        let (kind, key) = target(uri)?;
        let native = tokio::select! {
            result = read(&self.0.services.home_assistant, kind, &key) => result.map_err(|_| ServerError::resource_not_found("unavailable authoring resource"))?,
            () = context.cancelled() => return Err(ServerError::internal("resource read cancelled")),
        };
        let mut result = mcp::McpResourceResult::text(
            uri,
            if kind == "blueprint" {
                "application/yaml"
            } else {
                "application/json"
            },
            &native,
        );
        result.raw["contents"][0]["_meta"] = json!({"revision":digest(&native),"editable":true,"concurrency":"single-writer","revision_check":"best-effort"});
        bounded_text_resource(result)
    }

    pub(super) async fn author(
        &self,
        name: &str,
        args: &serde_json::Value,
        context: ServerContext,
    ) -> ServerResult<McpToolResult> {
        let payload = payload(name, args)?;
        let kind = payload["kind"].as_str().unwrap();
        let key = payload["key"].as_str().unwrap();
        let client = &self.0.services.home_assistant;
        let observed = tokio::select! {
            result = preflight(client, kind, key, name == "create") => result,
            () = context.cancelled() => return Ok(failure("cancelled")),
        };
        let initial = if name == "create" {
            match observed {
                Ok(_) => return Ok(failure("already_exists")),
                Err(Error::ConfigNotFound) => None,
                Err(error) => {
                    return Ok(error
                        .into_tool_error("authoring preflight")
                        .into_mcp_result());
                }
            }
        } else {
            match observed {
                Ok(native) if digest(&native) == payload["expected_revision"] => Some(native),
                Ok(_) => return Ok(failure("revision_conflict")),
                Err(error) => {
                    return Ok(error
                        .into_tool_error("authoring preflight")
                        .into_mcp_result());
                }
            }
        };
        let native = if let Some(initial) = &initial {
            apply_edits(initial, payload["edits"].as_array().unwrap())?
        } else {
            payload["text"].as_str().unwrap().to_owned()
        };
        let command = candidate(kind, key, &native)?;
        if let Some(initial) = &initial {
            let unchanged = match &command {
                Candidate::Config(command) => {
                    let previous: serde_json::Value =
                        serde_json::from_str(initial).map_err(|_| invalid())?;
                    command.config == previous
                }
                Candidate::Blueprint { semantic, .. } => *semantic == blueprint_semantic(initial)?,
            };
            if unchanged {
                return Err(invalid());
            }
        }
        let reread = tokio::select! {
            result = preflight(client, kind, key, name == "create") => result,
            () = context.cancelled() => return Ok(failure("cancelled")),
        };
        match (&initial, reread) {
            (Some(initial), Ok(current)) if digest(initial) == digest(&current) => {}
            (None, Err(Error::ConfigNotFound)) => {}
            (None, Ok(_)) => return Ok(failure("already_exists")),
            (Some(_), Ok(_) | Err(Error::ConfigNotFound)) => {
                return Ok(failure("revision_conflict"));
            }
            (_, Err(error)) => {
                return Ok(error
                    .into_tool_error("authoring preflight")
                    .into_mcp_result());
            }
        }
        let write = async {
            match &command {
                Candidate::Config(command) if kind == "scene" => client.upsert_scene(command).await,
                Candidate::Config(command) => client.upsert_automation(command).await,
                Candidate::Blueprint { command, .. } => {
                    client
                        .save_blueprint_authoring(command, name != "create")
                        .await
                }
            }
        };
        let result = tokio::select! {
            biased;
            () = context.cancelled() => return Ok(failure("mutation_outcome_unknown")),
            result = write => result,
        };
        match result {
            Ok(value) if value["accepted"] == true => Ok(McpToolResult::new(
                json!({"content":[{"type":"text","text":"Native persistence acknowledged; reload completion is not implied."}],"structuredContent":{"accepted":true,"reload_complete":false,"concurrency":"single-writer","revision_check":"best-effort"}}),
            )),
            Err(
                Error::InvalidArguments
                | Error::CapacityExhausted
                | Error::Unauthorized
                | Error::RequestRejected,
            ) => Ok(failure("request_rejected")),
            _ => Ok(failure("mutation_outcome_unknown")),
        }
    }
}

pub(super) fn tools() -> Vec<mcp::McpToolDefinition> {
    let string = json!({"type":"string","maxLength":ITEM_LIMIT});
    let replace = json!({"type":"object","additionalProperties":false,"required":["operation","old_text","new_text"],"properties":{"operation":{"const":"replace"},"old_text":{"type":"string","minLength":1,"maxLength":ITEM_LIMIT},"new_text":string}});
    let inserts = [false,true].map(|anchored| {
        let mut schema = json!({"type":"object","additionalProperties":false,"required":["operation","text","placement"],"properties":{"operation":{"const":"insert"},"text":string,"placement":{"enum":if anchored {vec!["before","after"]} else {vec!["start","end"]}}}});
        if anchored { schema["required"].as_array_mut().unwrap().push(json!("anchor")); schema["properties"]["anchor"] = json!({"type":"string","minLength":1,"maxLength":ITEM_LIMIT}); }
        schema
    });
    let edit = json!({"type":"object","additionalProperties":false,"required":["uri","expected_revision","edits"],"properties":{"uri":{"type":"string"},"expected_revision":{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"},"edits":{"type":"array","minItems":1,"maxItems":256,"items":{"oneOf":[replace,inserts[0],inserts[1]]}}}});
    let inputs = ["config_key","path"].map(|field| json!({"type":"object","additionalProperties":false,"required":[field,"text"],"properties":{field:{"type":"string","minLength":1},"text":{"type":"string","minLength":1,"maxLength":ITEM_LIMIT}}}));
    let create = json!({"type":"object","additionalProperties":false,"required":["action","input"],"properties":{"action":{"enum":["scene.create","automation.create","blueprint.create"]},"input":{"oneOf":inputs}}});
    [("create",create),("edit",edit)].into_iter().map(|(name,schema)| {
        let mut tool = mcp::progressive::tool_definition(name,"Administrator-only read-then-write native authoring. Single writer only; revision/existence checks are best-effort, not atomic. Never retry an unknown mutation outcome.",schema);
        tool.annotations = Some(json!({"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":true}));
        tool
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blueprint_semantics_preserve_input_tags_and_reject_malformed_trees() {
        let tagged = "blueprint:\n  name: One\n  domain: automation\naction:\n  target: !input chosen_entity\n";
        let reordered = "# comment\naction: {target: !input chosen_entity}\nblueprint: {domain: automation, name: One}\n";
        assert_eq!(
            blueprint_semantic(tagged).unwrap(),
            blueprint_semantic(reordered).unwrap()
        );
        let semantic = blueprint_semantic(tagged).unwrap();
        assert!(
            matches!(&semantic["action"]["target"], serde_yaml_ng::Value::Tagged(value) if value.tag == "input" && value.value == "chosen_entity")
        );
        assert_ne!(
            semantic,
            blueprint_semantic(&tagged.replace("chosen_entity", "another_entity")).unwrap()
        );
        for invalid_yaml in [
            "[one, two]",
            "null",
            "scalar",
            "blueprint: [broken",
            "---\na: 1\n---\nb: 2\n",
            "a: 1\na: 2\n",
            "a:\n  b: 1\n  b: 2\n",
            "a: *missing\n",
        ] {
            assert!(
                candidate("blueprint", "local/test.yaml", invalid_yaml).is_err(),
                "{invalid_yaml}"
            );
        }
        let deep = format!("a: {}0{}", "[".repeat(256), "]".repeat(256));
        assert!(blueprint_semantic(&deep).is_err());
        assert!(blueprint_semantic(&"a".repeat(ITEM_LIMIT + 1)).is_err());
        assert!(
            candidate(
                "blueprint",
                "local/test.yaml",
                "unknown_native_schema: true\n"
            )
            .is_ok()
        );
    }

    #[test]
    fn edits_are_ordered_overlap_aware_and_bounded() {
        let edits = vec![
            json!({"operation":"replace","old_text":"one","new_text":"two"}),
            json!({"operation":"insert","placement":"after","anchor":"two","text":"!"}),
        ];
        assert_eq!(apply_edits("one", &edits).unwrap(), "two!");
        for (initial, edits) in [
            (
                "aaa".to_owned(),
                vec![json!({"operation":"replace","old_text":"aa","new_text":"b"})],
            ),
            (
                "x x".to_owned(),
                vec![json!({"operation":"insert","placement":"before","anchor":"x","text":"b"})],
            ),
            (
                "x".to_owned(),
                vec![json!({"operation":"replace","old_text":"absent","new_text":"b"})],
            ),
            (
                "x".to_owned(),
                vec![json!({"operation":"replace","old_text":"x","new_text":"x"})],
            ),
            (
                "x".to_owned(),
                vec![json!({"operation":"insert","placement":"end","text":""})],
            ),
            (
                "x".to_owned(),
                vec![
                    json!({"operation":"replace","old_text":"x","new_text":"y"}),
                    json!({"operation":"replace","old_text":"y","new_text":"x"}),
                ],
            ),
            (
                "x".repeat(ITEM_LIMIT),
                vec![json!({"operation":"insert","placement":"end","text":"x"})],
            ),
            (
                "x".repeat(ITEM_LIMIT + 1),
                vec![json!({"operation":"replace","old_text":"x","new_text":""})],
            ),
        ] {
            assert!(apply_edits(&initial, &edits).is_err());
        }
        assert_eq!(
            apply_edits(
                "a\u{03bb}b",
                &[json!({"operation":"insert","placement":"after","anchor":"\u{03bb}","text":"c"})]
            )
            .unwrap(),
            "a\u{03bb}cb"
        );
    }

    #[test]
    fn candidates_validate_native_ids_and_closed_schema_bounds() {
        assert!(candidate("scene", "x", "{\"id\":\"wrong\"}").is_err());
        assert!(candidate("scene", "x", "[]").is_err());
        assert!(candidate("scene", "x", "{broken private source}").is_err());
        assert!(candidate("blueprint", "../x.yaml", "x").is_err());
        let revision = digest("x");
        for edits in [
            vec![],
            vec![json!({"operation":"insert","placement":"start","text":"x","anchor":null})],
            vec![json!({"operation":"replace","old_text":"x","new_text":"y","secret":"private"})],
            vec![json!({"operation":"replace","old_text":"","new_text":"y"})],
            vec![json!({"operation":"insert","placement":"after","text":"x"})],
            vec![json!({"operation":"insert","placement":"end","text":"x"}); 257],
        ] {
            assert!(
                payload(
                    "edit",
                    &json!({"uri":"smarthome://scenes/x","expected_revision":revision,"edits":edits})
                )
                .is_err()
            );
        }
        for uri in [
            "smarthome://scenes/x?filter=a",
            "smarthome://scenes/x#f",
            "smarthome://scenes/%78",
            "smarthome://scenes/x/",
            "smarthome://blueprints/a/b.yaml",
            "smarthome://blueprints/a%2fb.yaml",
        ] {
            assert!(target(uri).is_err(), "{uri}");
        }
    }
    #[test]
    fn direct_edits_are_closed_and_preserved() {
        let mut args = json!({"uri":"smarthome://scenes/a","expected_revision":format!("sha256:{}","a".repeat(64)),"edits":[{"operation":"insert","placement":"start","text":"x"}]});
        assert_eq!(payload("edit", &args).unwrap()["edits"], args["edits"]);
        args["edits"][0]["anchor"] = serde_json::Value::Null;
        assert!(payload("edit", &args).is_err());
        assert!(payload("edit", &json!({"action":"scene.edit","input":args})).is_err());
    }
}
