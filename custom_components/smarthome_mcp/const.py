"""Constants for Smarthome MCP."""

from typing import Final

DOMAIN: Final = "smarthome_mcp"
NAME: Final = "Smarthome MCP"
WS_COMMAND: Final = f"{DOMAIN}/blueprint/get"

MAX_PATH_LENGTH: Final = 512
MAX_SEGMENT_LENGTH: Final = 128
MAX_YAML_BYTES: Final = 256 * 1024
