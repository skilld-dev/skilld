use skilld_native::upgrade_ui::{UpgradeChoice, UpgradeKey, UpgradePrompt, render_snapshot};

#[test]
fn the_prompt_accepts_defers_and_dismisses() {
    let mut prompt = UpgradePrompt::default();
    assert_eq!(
        prompt.update(UpgradeKey::Confirm),
        Some(UpgradeChoice::Upgrade)
    );
    let mut prompt = UpgradePrompt::default();
    assert_eq!(prompt.update(UpgradeKey::Down), None);
    assert_eq!(
        prompt.update(UpgradeKey::Confirm),
        Some(UpgradeChoice::Later)
    );
    let mut prompt = UpgradePrompt::default();
    assert_eq!(prompt.update(UpgradeKey::Up), None);
    assert_eq!(
        prompt.update(UpgradeKey::Confirm),
        Some(UpgradeChoice::Dismiss)
    );
    assert_eq!(
        prompt.update(UpgradeKey::Cancel),
        Some(UpgradeChoice::Later)
    );
}

#[test]
fn the_prompt_shows_versions_action_and_choices_without_color() {
    let output = render_snapshot(
        "3.6.7",
        "3.7.0",
        "Run pnpm add --global skilld@3.7.0",
        80,
        16,
    );
    println!("{output}");
    assert!(
        output.contains("Upgrade available: 3.6.7 -> 3.7.0"),
        "{output}"
    );
    assert!(
        output.contains("Run pnpm add --global skilld@3.7.0"),
        "{output}"
    );
    assert!(output.contains("> Upgrade now"), "{output}");
    assert!(output.contains("Not now"), "{output}");
    assert!(
        output.contains("Don't remind me for this version"),
        "{output}"
    );
    assert!(output.contains("Enter to confirm"), "{output}");
    assert!(!output.contains('\x1b'));
}

#[test]
fn the_prompt_fits_a_narrow_terminal() {
    let output = render_snapshot(
        "3.6.7",
        "3.7.0",
        "Run pnpm add --global skilld@3.7.0",
        40,
        16,
    );
    assert!(output.contains("Upgrade now"), "{output}");
    assert!(
        output.contains("Don't remind me for this version"),
        "{output}"
    );
}
