use skilld_command::upgrade::{
    InstallChannel, PackageRunner, UpgradeNotice, UpgradePlan, UpgradeState, UpgradeWorker,
    parse_github_release, parse_npm_latest, plan_upgrade, release_asset,
};

const DAY: u64 = 86_400;
const NOW: u64 = 1_800_000_000;

#[test]
fn a_known_standalone_upgrade_never_installs_without_an_answer() {
    let plan = plan_upgrade(
        "3.0.0",
        &InstallChannel::Standalone,
        &state(NOW - 60, Some("3.1.0")),
        NOW,
    );

    assert_eq!(plan.worker, None);
    assert!(plan.notice.is_some());
}

fn state(checked_at: u64, latest: Option<&str>) -> UpgradeState {
    UpgradeState {
        checked_at,
        latest: latest.map(str::to_owned),
        last_error: None,
    }
}

#[test]
fn an_unmanaged_binary_never_checks_or_upgrades() {
    let plan = plan_upgrade(
        "3.0.0",
        &InstallChannel::Unmanaged,
        &state(0, Some("9.0.0")),
        NOW,
    );

    assert_eq!(plan, UpgradePlan::default());
}

#[test]
fn a_stale_check_refreshes_in_the_background_without_a_notice() {
    let channel = InstallChannel::Npm(PackageRunner::Npm);

    assert_eq!(
        plan_upgrade("3.0.0", &channel, &UpgradeState::default(), NOW),
        UpgradePlan {
            worker: Some(UpgradeWorker::Check),
            notice: None,
        }
    );
    assert_eq!(
        plan_upgrade("3.0.0", &channel, &state(NOW - 60, Some("3.0.0")), NOW),
        UpgradePlan::default()
    );
    assert_eq!(
        plan_upgrade("3.0.0", &channel, &state(NOW + DAY, Some("3.0.0")), NOW).worker,
        Some(UpgradeWorker::Check),
        "a check time from the future counts as stale"
    );
}

#[test]
fn a_cached_newer_release_offers_an_upgrade_for_every_managed_channel() {
    let fresh = state(NOW - 60, Some("3.1.0"));
    for channel in [
        InstallChannel::Standalone,
        InstallChannel::Npm(PackageRunner::Npx),
        InstallChannel::Npm(PackageRunner::Npm),
        InstallChannel::Npm(PackageRunner::Pnpm),
        InstallChannel::Npm(PackageRunner::Yarn),
        InstallChannel::Npm(PackageRunner::Bun),
    ] {
        let plan = plan_upgrade("3.0.0", &channel, &fresh, NOW);
        assert_eq!(plan.worker, None);
        assert_eq!(
            plan.notice,
            Some(UpgradeNotice {
                version: "3.1.0".to_owned()
            })
        );
    }
}

#[test]
fn versions_compare_by_number_and_never_downgrade() {
    let channel = InstallChannel::Npm(PackageRunner::Npm);
    for (current, latest, newer) in [
        ("3.0.0", "3.0.10", true),
        ("3.0.9", "3.0.10", true),
        ("3.0.0-beta.5", "3.0.0", true),
        ("3.0.0", "3.0.0-beta.9", false),
        ("3.1.0", "3.0.9", false),
        ("3.0.0", "3.0.0", false),
        ("3.0.0", "not-a-version", false),
    ] {
        let plan = plan_upgrade(current, &channel, &state(NOW - 60, Some(latest)), NOW);
        assert_eq!(plan.notice.is_some(), newer, "{current} -> {latest}");
    }
}

#[test]
fn package_runners_parse_from_the_loader_value() {
    assert_eq!(PackageRunner::parse("npx"), Some(PackageRunner::Npx));
    assert_eq!(PackageRunner::parse("pnpm"), Some(PackageRunner::Pnpm));
    assert_eq!(PackageRunner::parse("deno"), None);
}

#[test]
fn every_release_target_has_one_asset_name() {
    assert_eq!(
        release_asset("linux", "x86_64", "gnu"),
        Some("skilld-cli-linux-x64-gnu")
    );
    assert_eq!(
        release_asset("linux", "aarch64", "musl"),
        Some("skilld-cli-linux-arm64-musl")
    );
    assert_eq!(
        release_asset("macos", "aarch64", ""),
        Some("skilld-cli-darwin-arm64")
    );
    assert_eq!(
        release_asset("windows", "x86_64", "msvc"),
        Some("skilld-cli-win32-x64-msvc.exe")
    );
    assert_eq!(release_asset("freebsd", "x86_64", ""), None);
}

#[test]
fn latest_versions_parse_from_github_and_npm_and_reject_anything_else() {
    assert_eq!(
        parse_github_release(br#"{"tag_name":"v3.1.0","draft":false}"#),
        Some("3.1.0".to_owned())
    );
    assert_eq!(
        parse_npm_latest(br#"{"name":"skilld","version":"3.1.0"}"#),
        Some("3.1.0".to_owned())
    );
    assert_eq!(
        parse_github_release(br#"{"tag_name":"v3.1.0/../../x"}"#),
        None
    );
    assert_eq!(parse_npm_latest(b"not json"), None);
}
