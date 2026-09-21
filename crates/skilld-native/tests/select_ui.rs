use skilld_core::{ListedOrigin, ListedSkill, MultiSkillRef, SkillListing};
use skilld_native::select_ui::{PickerKey, PickerModel, PickerOutcome, SkillChoice, choices_for};

fn choice(label: &str, selected: bool) -> SkillChoice {
    SkillChoice {
        label: label.to_owned(),
        description: None,
        selected,
    }
}

fn model() -> PickerModel {
    PickerModel::new(
        "vuejs/core names 3 Skills.",
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
    assert_eq!(model.header(), "vuejs/core names 3 Skills. Chosen 2 of 3.");
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
fn every_listed_skill_starts_chosen() {
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
            origin: ListedOrigin::Registry,
        }],
    };

    let choices = choices_for(&listing);

    assert_eq!(
        choices,
        [SkillChoice {
            label: "vue".to_owned(),
            description: Some("Build Vue interfaces.".to_owned()),
            selected: true,
        }]
    );
}
