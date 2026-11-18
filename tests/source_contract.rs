#[test]
fn reusable_component_depends_on_typed_tellus_port_not_mcp() {
    let source = include_str!("../src/lib.rs");
    assert!(source.contains("trait TellusPort"));
    assert!(!source.contains("jsonrpc"));
    assert!(!source.contains("tools/call"));
    assert!(!source.contains("ratatui::run"));
    assert!(!source.contains("std::env::args"));
}
