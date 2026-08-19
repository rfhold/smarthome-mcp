"""Lifecycle tests for Smarthome MCP."""

from homeassistant.core import HomeAssistant
from homeassistant.setup import async_setup_component
from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.smarthome_mcp.const import DOMAIN, WS_COMMAND


async def test_command_follows_entry_lifecycle(hass: HomeAssistant) -> None:
    """The command exists only while the activation entry is loaded."""
    assert await async_setup_component(hass, DOMAIN, {})
    assert WS_COMMAND not in hass.data.get("websocket_api", {})

    entry = MockConfigEntry(domain=DOMAIN, data={})
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    assert WS_COMMAND in hass.data["websocket_api"]

    handler = hass.data["websocket_api"][WS_COMMAND]
    assert await hass.config_entries.async_reload(entry.entry_id)
    assert WS_COMMAND in hass.data["websocket_api"]
    assert hass.data["websocket_api"][WS_COMMAND] is not handler

    assert await hass.config_entries.async_unload(entry.entry_id)
    assert WS_COMMAND not in hass.data["websocket_api"]
