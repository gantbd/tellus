# Tellus

Tellus is Dendrite's composable human presentation peer. Named for the Roman
personification of the earth, it presents the grounded evidence beneath the
wider suite without owning that evidence. Its public
`GroundingPort` uses closed versioned observations rather than MCP tool calls,
prompt text, or diagnostic rendering. The reusable component owns projection
and local refresh only; the separate executable owns terminal lifecycle,
configuration, and headless JSON output.

The selected local adapter accepts only loopback endpoints, bounds response
bytes and entry counts, distinguishes pre-send from outcome-unknown transport
failure, and carries an optional bearer token obtained from Dendrite's existing
authentication owner.

Run the ordinary source qualification with:

```sh
make check
```

The optional `make qualify-dendrite` profile starts a real local Dendrite
fixture. It is separate from the public source gate because Dendrite and Rune
must be supplied as sibling source repositories.

