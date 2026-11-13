use annorex_hosts::Host;

#[test]
fn display_name_is_the_product_name() {
    assert_eq!(Host::ClaudeCode.display_name(), "Claude Code");
    assert_eq!(Host::Pi.display_name(), "Pi");
    assert_eq!(Host::OpenCode.display_name(), "OpenCode");
}
