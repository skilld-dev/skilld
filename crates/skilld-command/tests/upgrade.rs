use skilld_command::upgrade::{
    InstallChannel, PackageRunner, UpgradeNotice, UpgradePlan, UpgradeState, UpgradeWorker,
    parse_github_release, parse_npm_latest, plan_upgrade, release_asset,
};

const DAY: u64 = 86_400;
const NOW: u64 = 1_800_000_000;

fn state(checked_at: u64, latest: Option<&str>) -> UpgradeState {
    UpgradeState {
        checked_at,
        latest: latest.map(str::to_owned),
        attempted_version: None,
        attempted_at: None,
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
fn an_npm_install_names_the_upgrade_command_for_its_runner() {
    let fresh = state(NOW - 60, Some("3.1.0"));
    for (runner, command) in [
        (PackageRunner::Npx, "npx skilld@latest"),
        (PackageRunner::Npm, "npm install --global skilld"),
        (PackageRunner::Pnpm, "pnpm add --global skilld"),
        (PackageRunner::Yarn, "yarn global add skilld"),
        (PackageRunner::Bun, "bun add --global skilld"),
    ] {
        let plan = plan_upgrade("3.0.0", &InstallChannel::Npm(runner), &fresh, NOW);

        assert_eq!(plan.worker, None);
        let notice = plan.notice.expect("a newer version shows a notice");
        assert_eq!(
            notice,
            UpgradeNotice::Available {
                version: "3.1.0".to_owned(),
                command: command.to_owned(),
            }
        );
        assert_eq!(
            notice.message(),
            format!("skilld 3.1.0 is available. Run {command} to upgrade.")
        );
    }
}

#[test]
fn a_standalone_install_upgrades_itself_and_asks_for_a_restart() {
    let plan = plan_upgrade(
        "3.0.0",
        &InstallChannel::Standalone,
        &state(NOW - 60, Some("3.1.0")),
        NOW,
    );

    assert_eq!(
        plan.worker,
        Some(UpgradeWorker::Install {
            version: "3.1.0".to_owned()
        })
    );
    assert_eq!(
        plan.notice.unwrap().message(),
        "Upgrading skilld to 3.1.0 in the background. Restart skilld to use it."
    );
}

#[test]
fn a_recent_failed_upgrade_waits_before_it_retries() {
    let mut recent = state(NOW - 60, Some("3.1.0"));
    recent.attempted_version = Some("3.1.0".to_owned());
    recent.attempted_at = Some(NOW - 60);

    assert_eq!(
        plan_upgrade("3.0.0", &InstallChannel::Standalone, &recent, NOW),
        UpgradePlan::default()
    );

    recent.attempted_at = Some(NOW - DAY);
    assert!(matches!(
        plan_upgrade("3.0.0", &InstallChannel::Standalone, &recent, NOW).worker,
        Some(UpgradeWorker::Install { .. })
    ));
}

#[test]
fn a_retry_after_a_failed_upgrade_names_the_failure_in_its_notice() {
    let mut failed = state(NOW - 60, Some("3.1.0"));
    failed.attempted_version = Some("3.1.0".to_owned());
    failed.attempted_at = Some(NOW - DAY);
    failed.last_error = Some("UPGRADE_DOWNLOAD_FAILED".to_owned());

    let plan = plan_upgrade("3.0.0", &InstallChannel::Standalone, &failed, NOW);

    assert_eq!(
        plan.notice.map(|notice| notice.message()),
        Some(
            "The last upgrade attempt failed: UPGRADE_DOWNLOAD_FAILED. \
             Retrying skilld 3.1.0 in the background. Restart skilld to use it."
                .to_owned()
        )
    );
}

#[test]
fn a_first_install_ignores_a_failure_from_another_version() {
    let mut failed = state(NOW - 60, Some("3.2.0"));
    failed.attempted_version = Some("3.1.0".to_owned());
    failed.attempted_at = Some(NOW - DAY);
    failed.last_error = Some("UPGRADE_DOWNLOAD_FAILED".to_owned());

    let plan = plan_upgrade("3.0.0", &InstallChannel::Standalone, &failed, NOW);

    assert_eq!(
        plan.notice,
        Some(UpgradeNotice::Installing {
            version: "3.2.0".to_owned(),
            last_error: None,
        })
    );
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
