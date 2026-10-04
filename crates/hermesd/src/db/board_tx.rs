//! One board transaction. Every guarded board change reads what its guards
//! need and writes inside one `BEGIN IMMEDIATE` transaction that holds the
//! connection (ARCH-R4 §3), so a WIP count, a blocker's state or the actor's
//! roles can't change between the check and the write. The reads and writes
//! on `BoardTx` are the repository; the `Db` methods of the same names run one
//! of them in a transaction of its own, unguarded.

use rusqlite::{Connection, TransactionBehavior};

use crate::board::model::{Item, ItemComment, ItemLink, LinkKind};

use super::{Actor, Db, ItemEdit, MoveTo, NewItem, Write};

pub struct BoardTx<'a> {
    pub(super) conn: &'a Connection,
}

impl Db {
    /// Run `work` in one write transaction. An `Err` rolls everything back;
    /// a refusal that wrote nothing simply commits nothing.
    pub fn board_tx<T>(
        &self,
        work: impl FnOnce(&BoardTx<'_>) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = work(&BoardTx { conn: &tx })?;
        tx.commit()?;
        Ok(out)
    }

    /// Run `work` against one consistent snapshot, writing nothing.
    pub fn board_read<T>(
        &self,
        work: impl FnOnce(&BoardTx<'_>) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        work(&BoardTx { conn: &tx })
    }
}

/// `Db` methods that run the `BoardTx` method of the same name on their own.
macro_rules! one_shot {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)+) => {
        impl Db {
            $(pub fn $name(&self, $($arg: $ty),*) -> anyhow::Result<$ret> {
                self.board_tx(|t| t.$name($($arg),*))
            })+
        }
    };
}

one_shot! {
    fn create_item(new: &NewItem<'_>, actor: &Actor<'_>) -> Item;
    fn update_item(id: &str, expected: u64, edit: &ItemEdit<'_>, actor: &Actor<'_>) -> Write<Item>;
    fn move_item(id: &str, expected: u64, to: &MoveTo<'_>, actor: &Actor<'_>) -> Write<Item>;
    fn rank_item(
        id: &str, expected: u64, after_id: Option<&str>, before_id: Option<&str>, actor: &Actor<'_>
    ) -> Write<Item>;
    fn assign_item(id: &str, expected: u64, assignee: Option<&str>, actor: &Actor<'_>) -> Write<Item>;
    fn block_item(
        id: &str, expected: u64, block: Option<(Option<&str>, &str)>, actor: &Actor<'_>
    ) -> Write<Item>;
    fn add_item_comment(
        item_id: &str, body: &str, reply_to: Option<&str>, actor: &Actor<'_>
    ) -> ItemComment;
    fn add_item_link(
        item_id: &str, kind: LinkKind, target: &str, label: Option<&str>, actor: &Actor<'_>
    ) -> ItemLink;
    fn remove_item_link(item_id: &str, kind: LinkKind, target: &str, actor: &Actor<'_>) -> bool;
    fn check_ac(
        id: &str, expected: u64, idx: u32, checked: bool, machine: Option<&str>, actor: &Actor<'_>
    ) -> Write<Item>;
    fn item_links(item_id: &str) -> Vec<ItemLink>;
    fn item_project(id: &str) -> Option<String>;
}
