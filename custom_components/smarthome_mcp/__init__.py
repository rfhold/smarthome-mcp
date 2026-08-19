"""Smarthome MCP integration."""

from __future__ import annotations

from typing import Any

from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant

from .const import WS_COMMAND
from .websocket_api import async_register_command


async def async_setup(_hass: HomeAssistant, _config: dict[str, Any]) -> bool:
    """Set up the integration package."""
    return True


async def async_setup_entry(hass: HomeAssistant, _entry: ConfigEntry) -> bool:
    """Activate the blueprint reader."""
    async_register_command(hass)
    return True


async def async_unload_entry(hass: HomeAssistant, _entry: ConfigEntry) -> bool:
    """Deactivate the blueprint reader."""
    handlers = hass.data.get("websocket_api")
    if handlers is not None:
        handlers.pop(WS_COMMAND, None)
    return True
