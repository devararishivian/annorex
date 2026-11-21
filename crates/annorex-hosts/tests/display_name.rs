use annorex_hosts::Host;

#[test]
fn display_name_is_the_product_name() {
    assert_eq!(Host::ClaudeCode.display_name(), "Claude Code");
    assert_eq!(Host::Pi.display_name(), "Pi");
    assert_eq!(Host::OpenCode.display_name(), "OpenCode");
}

#[test]
fn codex_is_the_fourth_host() {
    assert_eq!(Host::Codex.display_name(), "Codex");
    assert_eq!(Host::ALL.len(), 4);
    assert_eq!(Host::ALL[3], Host::Codex);
}
