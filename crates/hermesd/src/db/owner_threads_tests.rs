//! Owner threads and pins in the database (H-128 D6, D7).

use super::tests::setup;
use super::*;

fn sender(kind: SenderKind, bot_id: Option<&str>, name: &str) -> Sender {
    Sender {
        kind,
        bot_id: bot_id.map(str::to_string),
        name: name.to_string(),
    }
}

/// The owner, the bot to the owner, another bot, and a daemon notice.
fn conversation(db: &Db, bot: &Bot) -> [Message; 4] {
    let conv = db.dm_conversation(&bot.id).unwrap().unwrap();
    let bob = db
        .create_bot(&bot.project_id, "bob", "", "", "", "/tmp/bob", "bob", None)
        .unwrap();
    let say = |s: Sender, body: &str| {
        db.insert_message(&conv.id, &s, MessageKind::Chat, body, None, None)
            .unwrap()
    };
    [
        say(sender(SenderKind::User, None, "user"), "status?"),
        say(
            sender(SenderKind::Bot, Some(&bot.id), &bot.name),
            "all green",
        ),
        say(sender(SenderKind::Bot, Some(&bob.id), "bob"), "a task"),
        say(sender(SenderKind::User, None, "system"), "renamed"),
    ]
}

#[test]
fn the_thread_is_the_owner_and_the_bot_only() {
    let (db, bot) = setup();
    let [owner, mine, ..] = conversation(&db, &bot);
    let (page, more) = db.owner_thread_page(&bot.id, None, 50).unwrap();
    let nums: Vec<i64> = page.iter().map(|m| m.num).collect();
    assert_eq!(nums, vec![owner.num, mine.num]);
    assert!(!more);
    let (page, more) = db.owner_thread_page(&bot.id, None, 1).unwrap();
    assert_eq!(page[0].num, mine.num);
    assert!(more, "an older message exists");
    assert_eq!(
        db.owner_thread_last(&bot.id).unwrap().unwrap().num,
        mine.num
    );
}

#[test]
fn unread_counts_the_bots_messages_after_the_read_mark() {
    let (db, bot) = setup();
    let [_, mine, ..] = conversation(&db, &bot);
    assert_eq!(db.owner_unread(&bot.id).unwrap(), 1);
    assert_eq!(db.mark_owner_read(&bot.id, mine.num).unwrap(), mine.num);
    assert_eq!(db.owner_unread(&bot.id).unwrap(), 0);
    // Read state never goes back.
    assert_eq!(db.mark_owner_read(&bot.id, 1).unwrap(), mine.num);
    assert_eq!(db.owner_last_read(&bot.id).unwrap(), mine.num);
}

#[test]
fn a_thread_question_closes_when_the_owner_writes_or_dismisses() {
    let (db, bot) = setup();
    let [_, mine, ..] = conversation(&db, &bot);
    let q = db
        .add_owner_question(&bot, Asked::Thread(mine.num), "alice asks: ship?")
        .unwrap();
    let card = db
        .add_owner_question(&bot, Asked::Card("H-1"), "alice asks on H-1: ok?")
        .unwrap();
    let open = db
        .open_owner_questions(Some(&bot.project_id), None)
        .unwrap();
    assert_eq!(open.len(), 2);
    assert_eq!(
        db.thread_questions(&bot.id).unwrap(),
        vec![(mine.num, q.id.clone())]
    );

    // Another bot or a daemon notice doesn't answer it; the owner does.
    let conv = db.dm_conversation(&bot.id).unwrap().unwrap();
    db.insert_message(
        &conv.id,
        &sender(SenderKind::User, None, "system"),
        MessageKind::Note,
        "x",
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        db.open_owner_questions(None, Some(&bot.id)).unwrap().len(),
        2
    );
    db.insert_message(
        &conv.id,
        &sender(SenderKind::User, None, "user"),
        MessageKind::Chat,
        "yes",
        None,
        None,
    )
    .unwrap();
    let open = db.open_owner_questions(None, Some(&bot.id)).unwrap();
    assert_eq!(
        open,
        vec![card.clone()],
        "the card's question waits for a comment"
    );

    assert!(db.dismiss_owner_question(&card.id).unwrap());
    assert!(!db.dismiss_owner_question(&card.id).unwrap());
    assert!(db.owner_question(&card.id).unwrap().is_none());
    assert!(db.open_owner_questions(None, None).unwrap().is_empty());
}

#[test]
fn a_pin_is_stored_once() {
    let (db, bot) = setup();
    assert!(!db.project_pinned(&bot.project_id).unwrap());
    assert!(db.set_project_pinned(&bot.project_id, true).unwrap());
    assert!(!db.set_project_pinned(&bot.project_id, true).unwrap());
    assert!(db.project_pinned(&bot.project_id).unwrap());
    assert!(db.set_project_pinned(&bot.project_id, false).unwrap());
    assert!(!db.project_pinned(&bot.project_id).unwrap());
}

/// H-192: a turn's answer is claimed once; a claim whose post failed is
/// given back so a later pass retries (ARCH S1), and a posted one stays.
#[test]
fn an_answer_claim_is_given_back_only_when_nothing_was_posted() {
    let (db, bot) = setup();
    assert!(db.claim_owner_answer(&bot.id, "t1").unwrap());
    assert!(
        !db.claim_owner_answer(&bot.id, "t1").unwrap(),
        "claimed once"
    );
    db.release_owner_answer(&bot.id, "t1").unwrap();
    assert!(db.claim_owner_answer(&bot.id, "t1").unwrap(), "free again");

    db.set_owner_answer_num(&bot.id, "t1", 7).unwrap();
    db.release_owner_answer(&bot.id, "t1").unwrap();
    assert!(
        !db.claim_owner_answer(&bot.id, "t1").unwrap(),
        "posted stays"
    );
    assert_eq!(db.owner_answers(&bot.id).unwrap().get("t1"), Some(&7));
}
