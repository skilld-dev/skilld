use skilld_core::{ListedOrigin, ListedSkill, MultiSkillRef, SkillListing};
use skilld_native::select_ui::{
    PickerKey, PickerModel, PickerOutcome, SkillChoice, choices_for, render_snapshot,
};

fn choice(label: &str, selected: bool) -> SkillChoice {
    SkillChoice {
        label: label.to_owned(),
        description: None,
        selected,
    }
}

fn model() -> PickerModel {
    PickerModel::new(
        "vuejs/core",
        vec![
            choice("alpha", true),
            choice("beta", true),
            choice("gamma", true),
        ],
    )
}

#[test]
fn space_toggles_the_skill_under_the_cursor() {
    let mut model = model();
    model.update(PickerKey::Down);
    model.update(PickerKey::Toggle);
    model.update(PickerKey::Confirm);

    assert_eq!(model.outcome(), Some(&PickerOutcome::Chose(vec![0, 2])));
    assert_eq!(model.header(), "vuejs/core names 3 Skills. 2 chosen.");
}

#[test]
fn toggle_all_clears_every_skill_then_selects_every_skill() {
    let mut model = model();
    model.update(PickerKey::ToggleAll);
    assert!(model.choices().iter().all(|choice| !choice.selected));

    model.update(PickerKey::ToggleAll);
    model.update(PickerKey::Confirm);

    assert_eq!(model.outcome(), Some(&PickerOutcome::Chose(vec![0, 1, 2])));
}

#[test]
fn the_cursor_wraps_at_both_ends() {
    let mut model = model();
    model.update(PickerKey::Up);
    assert_eq!(model.cursor(), 2);

    model.update(PickerKey::Down);
    assert_eq!(model.cursor(), 0);
}

#[test]
fn cancelling_chooses_no_skill() {
    let mut model = model();
    model.update(PickerKey::Cancel);

    assert_eq!(model.outcome(), Some(&PickerOutcome::Cancelled));
}

#[test]
fn no_listed_skill_starts_chosen() {
    let listing = SkillListing {
        reference: MultiSkillRef::Repository {
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
        },
        items: vec![ListedSkill {
            name: "vue".to_owned(),
            owner: "vuejs".to_owned(),
            repository: "core".to_owned(),
            description: Some("Build Vue interfaces.".to_owned()),
            origin: ListedOrigin::Registry { path: None },
        }],
    };

    let choices = choices_for(&listing);

    assert_eq!(
        choices,
        [SkillChoice {
            label: "vue".to_owned(),
            description: Some("Build Vue interfaces.".to_owned()),
            selected: false,
        }]
    );
}

#[test]
fn the_picker_aligns_names_and_cuts_a_description_that_does_not_fit() {
    let mut model = PickerModel::new(
        "skilld-dev/vue-ecosystem-skills",
        vec![
            SkillChoice {
                label: "floating-ui-vue-skilld".to_owned(),
                description: Some(
                    "Floating UI for Vue. ALWAYS use when writing code importing it.".to_owned(),
                ),
                selected: false,
            },
            SkillChoice {
                label: "pinia-skilld".to_owned(),
                description: Some("Intuitive, type safe and flexible Store for Vue.".to_owned()),
                selected: false,
            },
        ],
    );
    model.update(PickerKey::Toggle);

    let frame = render_snapshot(&model, 72, 8, false);

    assert_eq!(
        frame,
        [
            "skilld-dev/vue-ecosystem-skills names 2 Skills. 1 chosen",
            "",
            "\u{276f} \u{25cf} floating-ui-vue-skilld  Floating UI for Vue. ALWAYS use when writin\u{2026}",
            "  \u{25cb} pinia-skilld            Intuitive, type safe and flexible Store for\u{2026}",
            "",
            "",
            "",
            "space choose   a all   / filter   enter install   esc cancel",
        ]
        .join("\n")
    );
}

fn typing(model: &mut PickerModel, value: &str) {
    model.update(PickerKey::FilterStart);
    for character in value.chars() {
        model.update(PickerKey::FilterChar(character));
    }
}

fn named(names: &[&str]) -> PickerModel {
    PickerModel::new(
        "skilld-dev/vue-ecosystem-skills",
        names
            .iter()
            .map(|name| SkillChoice {
                label: (*name).to_owned(),
                description: Some(format!("The {name} Skill for Vue.")),
                selected: false,
            })
            .collect(),
    )
}

#[test]
fn the_filter_matches_a_name_or_a_description_ignoring_case() {
    let mut model = named(&["pinia-skilld", "pinia-colada-skilld", "vue-router-skilld"]);

    typing(&mut model, "PINIA");
    assert_eq!(model.visible(), [0, 1]);

    model.update(PickerKey::Cancel);
    typing(&mut model, "router Skill for");
    assert!(model.visible().is_empty());

    model.update(PickerKey::FilterBackspace);
    assert_eq!(model.filter(), "router Skill fo");
}

#[test]
fn choosing_all_takes_only_what_the_filter_shows() {
    let mut model = named(&["pinia-skilld", "pinia-colada-skilld", "vue-router-skilld"]);

    typing(&mut model, "pinia");
    model.update(PickerKey::ToggleAll);
    model.update(PickerKey::Cancel);
    model.update(PickerKey::Confirm);

    assert_eq!(model.outcome(), Some(&PickerOutcome::Chose(vec![0, 1])));
}

#[test]
fn escape_clears_the_filter_before_it_cancels_the_picker() {
    let mut model = named(&["pinia-skilld", "vue-router-skilld"]);
    typing(&mut model, "pinia");

    model.update(PickerKey::Cancel);

    assert_eq!(model.filter(), "");
    assert!(!model.filtering());
    assert_eq!(model.outcome(), None);

    model.update(PickerKey::Cancel);

    assert_eq!(model.outcome(), Some(&PickerOutcome::Cancelled));
}

#[test]
fn the_cursor_stays_inside_the_filtered_list() {
    let mut model = named(&["alpha-skilld", "beta-skilld", "gamma-skilld"]);
    model.update(PickerKey::Down);
    model.update(PickerKey::Down);
    assert_eq!(model.cursor(), 2);

    typing(&mut model, "beta");

    assert_eq!(model.cursor(), 0);
    model.update(PickerKey::Toggle);
    model.update(PickerKey::FilterDone);
    model.update(PickerKey::Confirm);

    assert_eq!(model.outcome(), Some(&PickerOutcome::Chose(vec![1])));
}

#[test]
fn a_filter_that_matches_nothing_says_so() {
    let mut model = named(&["pinia-skilld", "vue-router-skilld"]);
    typing(&mut model, "zz");

    let frame = render_snapshot(&model, 60, 7, false);

    assert_eq!(
        frame,
        [
            "skilld-dev/vue-ecosystem-skills names 2 Skills. 0 chosen, 0",
            "filter zz\u{2588}",
            "No Skill matches zz",
            "",
            "",
            "",
            "type to filter   enter keep it   esc clear",
        ]
        .join("\n")
    );
}
