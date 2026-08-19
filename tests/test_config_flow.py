"""Tests for the Smarthome MCP config flow."""

from homeassistant import config_entries, data_entry_flow
from homeassistant.core import HomeAssistant
from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.smarthome_mcp.const import DOMAIN, NAME


async def test_user_flow_creates_no_input_entry(hass: HomeAssistant) -> None:
    """The user flow immediately creates the activation entry."""
    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": config_entries.SOURCE_USER}
    )

    assert result["type"] is data_entry_flow.FlowResultType.CREATE_ENTRY
    assert result["title"] == NAME
    assert result["data"] == {}


async def test_user_flow_aborts_when_already_configured(
    hass: HomeAssistant,
) -> None:
    """A second entry cannot be configured."""
    MockConfigEntry(domain=DOMAIN, data={}).add_to_hass(hass)

    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": config_entries.SOURCE_USER}
    )

    assert result["type"] is data_entry_flow.FlowResultType.ABORT
    assert result["reason"] in {"already_configured", "single_instance_allowed"}
