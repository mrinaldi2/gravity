//! The PR reads the app makes (H-273; H-261 §9): `pr_list`, `pr_get`,
//! `pr_diff` and `pr_comments` as `hermes.pr.v1` answers. The board's home
//! answers them for its own WS clients and for a linked computer's, which
//! forwards them here (`pr_read`), so both see the same PRs. They read what
//! is recorded: the 5-minute watch notices unreported pushes, not a read.

use std::sync::Arc;

use bus::contract::pr::{self as p, pr_request::Request, pr_response::Response};

use crate::app::AppState;
use crate::decisions::{invalid, not_found};
use crate::prs::model::Pr;
use crate::prs::{comments, diff, states, wire};

/// The project a read is about.
pub fn project_of(request: &Request) -> Option<&str> {
    Some(match request {
        Request::PrList(r) => &r.project_id,
        Request::PrGet(r) => &r.project_id,
        Request::PrDiff(r) => &r.project_id,
        Request::PrComments(r) => &r.project_id,
        Request::CheckRerun(r) => &r.project_id,
        _ => return None,
    })
}

/// Whether `request` is one of the reads this module answers.
pub fn is_read(request: &Request) -> bool {
    matches!(
        request,
        Request::PrList(_) | Request::PrGet(_) | Request::PrDiff(_) | Request::PrComments(_)
    )
}

pub fn serve(app: &Arc<AppState>, request: Request) -> anyhow::Result<Response> {
    Ok(match request {
        Request::PrList(r) => {
            let prs = app
                .db
                .board_read(|t| t.prs(&r.project_id, &states(&r.states)))?;
            let prs = prs
                .iter()
                .map(|pr| {
                    Ok(wire::pull_request(
                        app.as_ref(),
                        &crate::prs::summary(app, pr)?,
                    ))
                })
                .collect::<anyhow::Result<_>>()?;
            Response::PrList(p::PrList { prs })
        }
        Request::PrGet(r) => {
            let pr = pr(app, &r.project_id, r.number)?;
            Response::Pr(wire::pull_request(
                app.as_ref(),
                &crate::prs::detail(app, &pr)?,
            ))
        }
        Request::PrDiff(r) => {
            let pr = pr(app, &r.project_id, r.number)?;
            Response::PrDiff(diff::serve(app, &pr, &r)?)
        }
        Request::PrComments(r) => {
            let pr = pr(app, &r.project_id, r.number)?;
            let sha = r.sha.clone().unwrap_or_else(|| pr.head_sha.clone());
            let shown = comments::list(app, &pr, Some(&sha))?;
            Response::PrComments(p::PrComments {
                sha,
                comments: shown
                    .iter()
                    .map(|c| wire::comment(app.as_ref(), c))
                    .collect(),
            })
        }
        _ => return Err(invalid("not a PR read")),
    })
}

/// A read for `project`: the home's own, for a read a linked computer sent.
pub fn for_project(mut request: Request, project: &str) -> Request {
    let id = match &mut request {
        Request::PrList(r) => &mut r.project_id,
        Request::PrGet(r) => &mut r.project_id,
        Request::PrDiff(r) => &mut r.project_id,
        Request::PrComments(r) => &mut r.project_id,
        _ => return request,
    };
    *id = project.to_string();
    request
}

/// The home's answer, its PRs named in this computer's `project`.
pub fn into_project(response: &mut p::PrResponse, project: &str) {
    match response.response.as_mut() {
        Some(Response::Pr(pr)) => pr.project_id = project.to_string(),
        Some(Response::PrList(list)) => {
            for pr in &mut list.prs {
                pr.project_id = project.to_string();
            }
        }
        _ => {}
    }
}

fn pr(app: &AppState, project: &str, number: u32) -> anyhow::Result<Pr> {
    app.db
        .board_read(|t| t.pr(project, number))?
        .ok_or_else(|| not_found(format!("no PR #{number} in this project")))
}
