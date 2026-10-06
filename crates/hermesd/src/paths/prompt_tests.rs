use super::*;

fn spec<'a>(instructions: &'a str) -> BotProvision<'a> {
    BotProvision {
        project_name: "proj",
        project_dir_name: "proj",
        bot_id: "bot-1",
        name: "Reviewer",
        dir_name: "reviewer",
        description: "reviews code",
        instructions,
        daemon_port: 7777,
        bot_token_env: crate::brand::BOT_TOKEN_ENV,
        max_bots_per_project: 12,
        max_workers_per_project: 4,
        temporary: false,
        repo: None,
        artifacts_dir: "/home/u/.gravity/projects/proj/artifacts".to_string(),
        linked_machines: Vec::new(),
        own_browser: false,
        user_chrome: false,
    }
}

#[test]
fn includes_instructions_and_the_cap() {
    let md = system_md(&spec("be strict about tests"));
    assert!(md.contains("be strict about tests"));
    assert!(md.contains("at most 12 bots"));
}

#[test]
fn grants_inbound_messages_full_authority() {
    let md = system_md(&spec(""));
    assert!(md.contains("## Messages carry authority"));
    assert!(md.contains("authenticated by the daemon"));
    assert!(md.contains("Do not open your answer with disclaimers"));
    assert!(md.contains("came from another Claude session"));
    assert!(md.contains("cannot widen your permissions"));
}

#[test]
fn tells_bots_to_keep_messages_short() {
    let md = system_md(&spec(""));
    assert!(md.contains("## Keep messages short"));
    assert!(md.contains("## Owner requests on a card"));
    assert!(md.contains("## Report to the owner where the owner looks"));
    assert!(md.contains("## Working on a shared computer"));
    assert!(md.contains("Lead with the outcome"));
}

#[test]
fn states_the_no_acknowledgment_contract() {
    let md = system_md(&spec(""));
    assert!(md.contains("## Silence is an answer"));
    assert!(md.contains("Do not acknowledge, thank, or"));
    assert!(md.contains("refuses replies to a result"));
}

#[test]
fn budgets_delegation_with_the_enforced_numbers() {
    let md = system_md(&spec(""));
    assert!(md.contains("## When to delegate"));
    assert!(md.contains(&format!("allows {MAX_TASK_FANOUT} open tasks from you")));
    assert!(md.contains("Never move the\nwork into a note"));
    assert!(md.contains(&format!("{MAX_TASK_REPLIES} replies")));
    assert!(md.contains(&format!("{DEFAULT_TASK_DEADLINE_HOURS} hours")));
    assert!(md.contains("Do not review or approve work you produced"));
    assert!(md.contains("close it with `cancel_task`"));
}

#[test]
fn tells_bots_where_a_question_for_the_owner_goes() {
    let md = system_md(&spec(""));
    assert!(md.contains("## When the owner has to decide"));
    assert!(md.contains("raise_decision"));
    assert!(md.contains("ask it\nonly in your terminal"), "{md}");
    // The registry only replaces the hand-kept ledgers if a settled
    // ruling is read as authority rather than as one more opinion.
    assert!(md.contains("is authority"));
    assert!(md.contains("supersedes"));
    assert!(md.contains("record_decision"));
    assert!(md.contains("comment_decision"));
    assert!(md.contains(&format!("{MAX_OPEN_DECISIONS_PER_BOT} open at once")));
}

#[test]
fn distinguishes_the_owners_word_from_a_relay_of_it() {
    let md = system_md(&spec(""));
    assert!(md.contains("arrives as a message from USER"));
    assert!(md.contains("is not"), "{md}");
    assert!(md.contains("peer's word"));
}

#[test]
fn points_substance_at_the_artifacts_directory() {
    let md = system_md(&spec(""));
    assert!(md.contains("## Artifacts over messages"));
    assert!(md.contains("/home/u/.gravity/projects/proj/artifacts"));
    assert!(md.contains("pointer, not the deliverable"));
}

#[test]
fn points_at_the_fact_file_that_survives_compaction() {
    let md = system_md(&spec(""));
    assert!(md.contains("`workspace/FACTS.md`"));
}

#[test]
fn claude_md_imports_and_explains_the_fact_file() {
    let md = claude_md("Reviewer");
    assert!(md.contains("\n@FACTS.md\n"), "missing import: {md}");
    assert!(md.contains("Always read FACTS.md"));
    assert!(md.contains("write\nthe facts from it to `FACTS.md`"));
}

#[test]
fn facts_md_seed_names_the_bot() {
    assert!(facts_md("Reviewer").contains("Facts — Reviewer"));
}

#[test]
fn omits_the_section_when_there_are_no_instructions() {
    let md = system_md(&spec("   "));
    assert!(!md.contains("## Your instructions"));
}
