"""Config flow for Smarthome MCP."""

from __future__ import annotations

from typing import Any

from homeassistant import config_entries

from .const import DOMAIN, NAME


class SmarthomeMcpConfigFlow(config_entries.ConfigFlow, domain=DOMAIN):
    """Create the single activation entry."""

    VERSION = 1

    async def async_step_user(
        self, _user_input: dict[str, Any] | None = None
    ) -> config_entries.ConfigFlowResult:
        """Create an entry without requesting input."""
        if self._async_current_entries():
            return self.async_abort(reason="already_configured")
        return self.async_create_entry(title=NAME, data={})
