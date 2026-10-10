//! The PR changes a board transaction notes as it writes (H-273), for the
//! pushes published once it commits (`prs::feed`). A check's change is also
//! one for each live PR whose head it counts on, the same tree included.

use rusqlite::{params, OptionalExtension};

use super::board_tx::BoardTx;
use crate::prs::feed::PrChange;
use crate::prs::model::Pr;

impl BoardTx<'_> {
    fn note(&self, change: PrChange) {
        self.changes.borrow_mut().push(change);
    }

    pub(super) fn note_pr(&self, pr: &Pr) {
        self.note(PrChange::Pr {
            project_id: pr.project_id.clone(),
            number: pr.number,
        });
    }

    pub(super) fn note_pr_id(&self, pr_id: &str) -> anyhow::Result<()> {
        let pr: Option<(String, u32)> = self
            .conn
            .query_row(
                "SELECT project_id, number FROM pr WHERE id = ?1",
                params![pr_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((project_id, number)) = pr {
            self.note(PrChange::Pr { project_id, number });
        }
        Ok(())
    }

    /// The PR of a review (`review`) or a line comment (`review_comment`).
    pub(super) fn note_pr_of(&self, table: PrPart, id: &str) -> anyhow::Result<()> {
        let sql = match table {
            PrPart::Review => "SELECT pr_id FROM review WHERE id = ?1",
            PrPart::Comment => "SELECT pr_id FROM review_comment WHERE id = ?1",
        };
        let pr_id: Option<String> = self
            .conn
            .query_row(sql, params![id], |r| r.get(0))
            .optional()?;
        match pr_id {
            Some(pr_id) => self.note_pr_id(&pr_id),
            None => Ok(()),
        }
    }

    pub(super) fn note_queue(&self, project_id: &str) {
        self.note(PrChange::Queue {
            project_id: project_id.to_string(),
        });
    }

    pub(super) fn note_queue_of(&self, pr_id: &str) -> anyhow::Result<()> {
        let project: Option<String> = self
            .conn
            .query_row(
                "SELECT project_id FROM pr WHERE id = ?1",
                params![pr_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(project) = project {
            self.note_queue(&project);
        }
        self.note_pr_id(pr_id)
    }

    pub(super) fn note_check_id(&self, id: &str) -> anyhow::Result<()> {
        let check: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT project_id, sha, name FROM check_run WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        match check {
            Some((project, sha, name)) => self.note_check(&project, &sha, &name),
            None => Ok(()),
        }
    }

    pub(super) fn note_check(&self, project_id: &str, sha: &str, name: &str) -> anyhow::Result<()> {
        self.note(PrChange::Check {
            project_id: project_id.to_string(),
            sha: sha.to_string(),
            name: name.to_string(),
        });
        let mut stmt = self.conn.prepare(
            "SELECT number FROM pr
             WHERE project_id = ?1 AND state IN ('open', 'merging') AND head_sha IN (
                SELECT h.sha FROM check_run h JOIN check_run c
                  ON h.project_id = c.project_id AND h.repo = c.repo AND h.tree = c.tree
                     AND h.name = c.name
                WHERE c.project_id = ?1 AND c.sha = ?2 AND c.name = ?3
                UNION SELECT ?2)",
        )?;
        let numbers = stmt
            .query_map(params![project_id, sha, name], |r| r.get::<_, u32>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for number in numbers {
            self.note(PrChange::Pr {
                project_id: project_id.to_string(),
                number,
            });
        }
        Ok(())
    }
}

/// A table whose rows belong to one PR.
#[derive(Clone, Copy)]
pub(super) enum PrPart {
    Review,
    Comment,
}
