"""Administrator-only blueprint WebSocket API."""

from __future__ import annotations

from typing import Any

import voluptuous as vol
from homeassistant.components import websocket_api
from homeassistant.components.blueprint.errors import (
    BlueprintException,
    InvalidBlueprint,
)
from homeassistant.components.websocket_api.connection import ActiveConnection
from homeassistant.components.websocket_api.decorators import (
    async_response,
    require_admin,
    websocket_command,
)
from homeassistant.core import HomeAssistant, callback

from .const import (
    MAX_PATH_LENGTH,
    MAX_SEGMENT_LENGTH,
    MAX_YAML_BYTES,
    WS_COMMAND,
)

_ALLOWED_FIELDS = {"id", "type", "path"}
_SAFE_SEGMENT_CHARS = frozenset(
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-."
)


def _valid_path(value: Any) -> bool:
    """Return whether value is a closed, safe automation blueprint path."""
    if not isinstance(value, str) or not value or len(value) > MAX_PATH_LENGTH:
        return False
    if not value.endswith(".yaml") or value.startswith(("/", "\\")):
        return False
    if any(ord(char) < 32 or ord(char) == 127 for char in value):
        return False

    segments = value.split("/")
    return all(
        segment not in {"", ".", ".."}
        and len(segment) <= MAX_SEGMENT_LENGTH
        and set(segment) <= _SAFE_SEGMENT_CHARS
        and "\\" not in segment
        for segment in segments
    )


@require_admin
@websocket_command(
    {
        vol.Required("type"): WS_COMMAND,
        vol.Optional("path"): object,
        vol.Extra: object,  # type: ignore[dict-item]
    }
)
@async_response
async def websocket_get_blueprint(
    hass: HomeAssistant,
    connection: ActiveConnection,
    msg: dict[str, Any],
) -> None:
    """Return bounded semantic YAML for an automation blueprint."""
    if set(msg) != _ALLOWED_FIELDS or not _valid_path(msg.get("path")):
        connection.send_error(
            msg["id"], "invalid_blueprint_path", "Invalid blueprint path"
        )
        return

    try:
        domain_blueprints = hass.data["blueprint"]["automation"]
        blueprint = await domain_blueprints.async_get_blueprint(msg["path"])
        yaml = blueprint.yaml()
    except InvalidBlueprint:
        connection.send_error(msg["id"], "invalid_blueprint", "Blueprint is invalid")
        return
    except BlueprintException:
        connection.send_error(
            msg["id"], "blueprint_not_found", "Blueprint is unavailable"
        )
        return
    except Exception:  # noqa: BLE001
        connection.send_error(msg["id"], "blueprint_error", "Unable to read blueprint")
        return

    if not isinstance(yaml, str):
        connection.send_error(msg["id"], "blueprint_error", "Unable to read blueprint")
        return
    if len(yaml.encode("utf-8")) > MAX_YAML_BYTES:
        connection.send_error(
            msg["id"], "blueprint_too_large", "Blueprint exceeds size limit"
        )
        return

    connection.send_result(msg["id"], {"yaml": yaml})


@callback
def async_register_command(hass: HomeAssistant) -> None:
    """Register the command once for the active entry."""
    handlers = hass.data.setdefault("websocket_api", {})
    if WS_COMMAND not in handlers:
        websocket_api.async_register_command(hass, websocket_get_blueprint)
