use skilld_core::{AGENT_TARGETS, AgentTargetId, DomainError, parse_agent_targets};

#[test]
fn every_agent_target_parses_and_serializes_by_its_id() {
    for target in AGENT_TARGETS {
        let id = target.id.as_str();
        assert_eq!(AgentTargetId::parse(id), Ok(target.id), "{id}");
        assert_eq!(
            serde_json::to_string(&target.id).unwrap(),
            format!("\"{id}\""),
            "{id}"
        );
        assert_eq!(
            serde_json::from_str::<AgentTargetId>(&format!("\"{id}\"")).unwrap(),
            target.id,
            "{id}"
        );
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
