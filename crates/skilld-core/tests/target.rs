use skilld_core::{AGENT_TARGETS, AgentTargetId, DomainError, parse_agent_targets};

#[test]
fn every_new_agent_target_parses_by_name() {
    let cases = [
        ("openclaw", AgentTargetId::Openclaw),
        ("hermes", AgentTargetId::Hermes),
        ("kiro", AgentTargetId::Kiro),
        ("kilo", AgentTargetId::Kilo),
        ("droid", AgentTargetId::Droid),
        ("trae", AgentTargetId::Trae),
        ("zed", AgentTargetId::Zed),
    ];
    for (name, expected) in cases {
        assert_eq!(AgentTargetId::parse(name), Ok(expected));
        assert_eq!(expected.as_str(), name);
    }
}

#[test]
fn all_expands_to_every_agent_target_in_registry_order() {
    let expected = AGENT_TARGETS
        .iter()
        .map(|target| target.id)
        .collect::<Vec<_>>();

    assert_eq!(
        parse_agent_targets(&["all".to_owned()]),
        Ok(expected.clone())
    );
    assert_eq!(
        parse_agent_targets(&["codex".to_owned(), "all".to_owned()]),
        Ok(expected)
    );
}

#[test]
fn all_with_an_invalid_value_fails_validation() {
    assert_eq!(
        parse_agent_targets(&["all".to_owned(), "nope".to_owned()]),
        Err(DomainError::InvalidTarget("nope".to_owned()))
    );
    assert_eq!(
        parse_agent_targets(&["nope".to_owned(), "all".to_owned()]),
        Err(DomainError::InvalidTarget("nope".to_owned()))
    );
}

#[test]
fn named_agent_targets_parse_in_the_given_order() {
    assert_eq!(
        parse_agent_targets(&["zed".to_owned(), "kiro".to_owned()]),
        Ok(vec![AgentTargetId::Zed, AgentTargetId::Kiro])
    );
    assert_eq!(
        parse_agent_targets(&["kiro".to_owned(), "nope".to_owned()]),
        Err(DomainError::InvalidTarget("nope".to_owned()))
    );
}
