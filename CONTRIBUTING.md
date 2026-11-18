# Contributing

Keep Tellus presentation-only. Grounding facts and authentication remain owned
by Dendrite; the reusable component must continue to depend on the typed
`TellusPort` rather than MCP envelopes or rendered diagnostic text.

Run `make check` before submitting a change. Use `make qualify-dendrite` when
the Dendrite and Rune sibling sources are available.
