"""Tests for the bounded blueprint WebSocket command."""

from __future__ import annotations

from typing import Any
from unittest.mock import AsyncMock, Mock

import pytest
from homeassistant.components.blueprint.errors import FailedToLoad, InvalidBlueprint
from homeassistant.core import HomeAssistant
from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.smarthome_mcp.const import DOMAIN, MAX_YAML_BYTES, WS_COMMAND


@pytest.fixture
async def loaded_entry(hass: HomeAssistant) -> MockConfigEntry:
    """Load the activation entry."""
    entry = MockConfigEntry(domain=DOMAIN, data={})
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    return entry


def _resolver(hass: HomeAssistant, outcome: Any) -> AsyncMock:
    resolver = AsyncMock()
    if isinstance(outcome, BaseException):
        resolver.async_get_blueprint.side_effect = outcome
    else:
        resolver.async_get_blueprint.return_value = outcome
    hass.data.setdefault("blueprint", {})["automation"] = resolver
    return resolver


async def _request(client: Any, path: Any = "safe/example.yaml", **extra: Any) -> dict:
    await client.send_json({"id": 1, "type": WS_COMMAND, "path": path, **extra})
    return await client.receive_json()


async def test_admin_receives_semantic_yaml(
    hass: HomeAssistant, hass_ws_client: Any, loaded_entry: MockConfigEntry
) -> None:
    """An administrator receives only serialized semantic YAML."""
    blueprint = Mock()
    blueprint.yaml.return_value = "blueprint:\n  domain: automation\n"
    resolver = _resolver(hass, blueprint)
    client = await hass_ws_client(hass)

    response = await _request(client)

    assert response["success"] is True
    assert response["result"] == {"yaml": "blueprint:\n  domain: automation\n"}
    resolver.async_get_blueprint.assert_awaited_once_with("safe/example.yaml")


async def test_non_admin_is_rejected(
    hass: HomeAssistant,
    hass_ws_client: Any,
    hass_read_only_access_token: str,
    loaded_entry: MockConfigEntry,
) -> None:
    """A non-administrator cannot read blueprint YAML."""
    client = await hass_ws_client(hass, hass_read_only_access_token)

    response = await _request(client)

    assert response["success"] is False
    assert response["error"]["code"] == "unauthorized"


@pytest.mark.parametrize(
    "path",
    [
        "../secret.yaml",
        "safe/../secret.yaml",
        "/absolute.yaml",
        "C:\\absolute.yaml",
        "safe//empty.yaml",
        "safe/./dot.yaml",
        "safe/control\n.yaml",
        "safe/example.yml",
        "safe/example.yaml/",
        "a" * 513,
        f"{'a' * 129}.yaml",
        "safe/non ascii.yaml",
        None,
        42,
    ],
)
async def test_invalid_paths_are_safe(
    hass: HomeAssistant,
    hass_ws_client: Any,
    loaded_entry: MockConfigEntry,
    path: Any,
) -> None:
    """Unsafe paths fail before reaching the Home Assistant resolver."""
    resolver = _resolver(hass, Mock())
    client = await hass_ws_client(hass)

    response = await _request(client, path)

    assert response["success"] is False
    assert response["error"] == {
        "code": "invalid_blueprint_path",
        "message": "Invalid blueprint path",
    }
    assert str(path) not in response["error"]["message"]
    resolver.async_get_blueprint.assert_not_awaited()


async def test_unknown_field_is_rejected_without_echo(
    hass: HomeAssistant, hass_ws_client: Any, loaded_entry: MockConfigEntry
) -> None:
    """The closed input rejects unknown data without reflecting it."""
    secret = "entity.secret_reference"
    client = await hass_ws_client(hass)

    response = await _request(client, unexpected=secret)

    assert response["error"]["code"] == "invalid_blueprint_path"
    assert secret not in str(response)


@pytest.mark.parametrize(
    ("failure", "code", "message"),
    [
        (
            FailedToLoad("automation", "private/path.yaml", FileNotFoundError()),
            "blueprint_not_found",
            "Blueprint is unavailable",
        ),
        (
            InvalidBlueprint("automation", "private/path.yaml", {}, "entity.secret"),
            "invalid_blueprint",
            "Blueprint is invalid",
        ),
        (
            RuntimeError("/config/private/path.yaml entity.secret"),
            "blueprint_error",
            "Unable to read blueprint",
        ),
    ],
)
async def test_upstream_errors_are_safe(
    hass: HomeAssistant,
    hass_ws_client: Any,
    loaded_entry: MockConfigEntry,
    failure: BaseException,
    code: str,
    message: str,
) -> None:
    """Missing, invalid, and internal failures do not leak upstream detail."""
    _resolver(hass, failure)
    client = await hass_ws_client(hass)

    response = await _request(client)

    assert response["error"] == {"code": code, "message": message}
    assert "private" not in str(response)
    assert "entity.secret" not in str(response)


async def test_oversized_yaml_is_rejected(
    hass: HomeAssistant, hass_ws_client: Any, loaded_entry: MockConfigEntry
) -> None:
    """Serialized UTF-8 YAML cannot exceed 256 KiB."""
    blueprint = Mock()
    blueprint.yaml.return_value = "x" * (MAX_YAML_BYTES + 1)
    _resolver(hass, blueprint)
    client = await hass_ws_client(hass)

    response = await _request(client)

    assert response["error"] == {
        "code": "blueprint_too_large",
        "message": "Blueprint exceeds size limit",
    }
    assert "x" * 100 not in str(response)
