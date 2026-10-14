use clevo_fan_control::config::CompatibilityConfig;

#[test]
fn default_compatibility_accepts_nh5xax_only_as_an_exact_model_name() {
    let compatibility = CompatibilityConfig::default();

    assert!(compatibility.matches_machine("NH5xAx").unwrap());
    assert!(compatibility.matches_machine("nh5xax").unwrap());
    assert!(!compatibility.matches_machine("NH5xAx2").unwrap());
    assert!(!compatibility.matches_machine("Other NH5xAx").unwrap());
}
