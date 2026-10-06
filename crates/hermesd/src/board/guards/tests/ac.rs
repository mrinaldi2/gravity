//! Acceptance criteria outside moves, and post-install ones (H-116, ARCH-R53).

use super::*;

#[test]
fn post_install_criteria_wait_for_done_not_for_the_package() {
    let mut c = case(Cat::Verify);
    c.item.release_id = Some("R-1".into());
    c.item.acceptance_criteria[0].checked = false;
    assert!(run(&c, Cat::Approval).contains(&"verify.ac".to_string()));
    c.item.acceptance_criteria[0].post_install = true;
    assert!(!run(&c, Cat::Approval).contains(&"verify.ac".to_string()));
    // Into Done, even the owner's by-hand close waits for it.
    c.item.release_id = None;
    c.reason = Some("shipped by hand");
    assert_eq!(run(&c, Cat::Done), ["done.post_install"]);
    c.item.acceptance_criteria[0].checked = true;
    assert!(run(&c, Cat::Done).is_empty());
}

#[test]
fn flagging_is_the_leads_or_the_assignees_before_verify() {
    let dev = bot("dev", &[Role::Dev]);
    assert!(flag_ac(&item(Cat::Doing), &dev, true).is_empty());
    assert_eq!(
        flag_ac(&item(Cat::Verify), &dev, true)[0].code,
        "role.not_allowed"
    );
    assert!(flag_ac(&item(Cat::Verify), &bot("lead", LEAD), true).is_empty());
    assert_eq!(
        flag_ac(&item(Cat::Done), &Who::Owner, true)[0].code,
        "ac.locked"
    );
    let other = bot("other", &[Role::Dev]);
    assert_eq!(
        flag_ac(&item(Cat::Doing), &other, true)[0].code,
        "role.not_allowed"
    );
    // The lead's check needs evidence; a tester's doesn't.
    let it = item(Cat::Verify);
    assert_eq!(
        check_ac(&it, &bot("lead", LEAD), None)[0].code,
        "ac.evidence"
    );
    assert!(check_ac(&it, &bot("lead", LEAD), Some("CI green")).is_empty());
    assert!(check_ac(&it, &bot("t", &[Role::Tester]), None).is_empty());
}

#[test]
fn a_flag_stays_once_the_package_is_submitted() {
    let lead = bot("lead", LEAD);
    for cat in [Cat::Approval, Cat::Deploying] {
        assert!(flag_ac(&item(cat), &lead, true).is_empty(), "{cat:?}");
        assert_eq!(
            flag_ac(&item(cat), &lead, false)[0].code,
            "ac.post_install_locked",
            "{cat:?}"
        );
    }
    assert!(flag_ac(&item(Cat::Verify), &lead, false).is_empty());
}
