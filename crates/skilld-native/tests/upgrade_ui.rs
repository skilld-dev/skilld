use skilld_native::upgrade_ui::{
    UpgradeChoice, UpgradeKey, UpgradePrompt, render_banner, render_snapshot,
};

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
        false,
    );
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
fn color_output_resets_styles_and_plain_banners_have_no_escape_codes() {
    let prompt = render_snapshot(
        "3.6.7",
        "3.7.0",
        "Run pnpm add --global skilld@3.7.0",
        80,
        13,
        true,
    );
    let banner = render_banner("3.6.7", "3.7.0", "Restart skilld to upgrade.", 80, true);
    println!("COLOR_PREVIEW_START\n{prompt}\n\n{banner}\nCOLOR_PREVIEW_END");
    assert!(
        prompt.contains("\x1b[7m"),
        "The selection must reverse inherited colors."
    );
    assert!(
        prompt.contains("\x1b[2m"),
        "Secondary text must dim the inherited foreground."
    );
    for output in [&prompt, &banner] {
        assert!(
            !output.contains(";2;"),
            "RGB overrides the terminal palette: {output}"
        );
        assert!(output.contains("\x1b[38;5;6m"), "{output}");
        assert!(output.lines().all(|line| line.ends_with("\x1b[0m")));
    }
    let plain = render_banner("3.6.7", "3.7.0", "Restart skilld to upgrade.", 40, false);
    assert!(plain.contains("Upgrade available: 3.6.7 -> 3.7.0"));
    assert!(!plain.contains('\x1b'));
}

#[test]
fn the_prompt_fits_a_narrow_terminal() {
    let output = render_snapshot(
        "3.6.7",
        "3.7.0",
        "Run pnpm add --global skilld@3.7.0",
        40,
        16,
        false,
    );
    assert!(output.contains("Upgrade now"), "{output}");
    assert!(
        output.contains("Don't remind me for this version"),
        "{output}"
    );
}

#[test]
fn a_short_prompt_keeps_the_action_and_consent_visible() {
    let output = render_snapshot("3.6.7", "3.7.0", "Run skilld upgrade", 40, 8, false);
    assert!(output.contains("Run skilld upgrade"), "{output}");
    assert!(output.contains("Upgrade now"), "{output}");
    assert!(output.contains("Enter"), "{output}");
    assert!(output.contains("Esc"), "{output}");
}

#[test]
fn an_unusable_prompt_does_not_hide_consent_behind_a_partial_form() {
    let output = render_snapshot("3.6.7", "3.7.0", "Run skilld upgrade", 12, 3, false);
    assert!(output.contains("Resize"), "{output}");
}
