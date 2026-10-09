//! A project's shared git repository.

use bus::{now, ProjectRepo};
use rusqlite::{params, OptionalExtension};

use super::{ts, Db};

impl Db {
    pub fn project_repo(&self, project_id: &str) -> anyhow::Result<Option<ProjectRepo>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT url, branch FROM project_repo WHERE project_id = ?1",
                params![project_id],
                |r| {
                    Ok(ProjectRepo {
                        url: r.get(0)?,
                        branch: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// The other repositories the owner lets this project's PRs use (H-266).
    pub fn extra_repos(&self, project_id: &str) -> anyhow::Result<Vec<String>> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT url FROM project_repo_extra WHERE project_id = ?1 ORDER BY url")?;
        let urls = stmt
            .query_map(params![project_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(urls)
    }

    /// Replaces that list; `by` is how the owner proved it (device or ticket).
    pub fn set_extra_repos(
        &self,
        project_id: &str,
        urls: &[String],
        by: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM project_repo_extra WHERE project_id = ?1",
            params![project_id],
        )?;
        for url in urls {
            tx.execute(
                "INSERT OR IGNORE INTO project_repo_extra(project_id, url, added_by, added_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![project_id, url, by, ts(now())],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Set the project's repository, or clear it with `None`.
    pub fn set_project_repo(
        &self,
        project_id: &str,
        repo: Option<&ProjectRepo>,
    ) -> anyhow::Result<()> {
        let conn = self.lock();
        match repo {
            Some(repo) => conn.execute(
                "INSERT INTO project_repo(project_id, url, branch, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(project_id) DO UPDATE
                   SET url = excluded.url, branch = excluded.branch,
                       updated_at = excluded.updated_at",
                params![project_id, repo.url, repo.branch, ts(now())],
            )?,
            None => conn.execute(
                "DELETE FROM project_repo WHERE project_id = ?1",
                params![project_id],
            )?,
        };
        Ok(())
    }
}
