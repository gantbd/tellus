# Grounding TUI

`grounding-tui` is Dendrite's composable human presentation peer. Its public
`GroundingPort` uses closed versioned observations rather than MCP tool calls,
prompt text, or diagnostic rendering. The reusable component owns projection
and local refresh only; the separate executable owns terminal lifecycle,
configuration, and headless JSON output.

The selected local adapter accepts only loopback endpoints, bounds response
bytes and entry counts, distinguishes pre-send from outcome-unknown transport
failure, and carries an optional bearer token obtained from Dendrite's existing
authentication owner.

