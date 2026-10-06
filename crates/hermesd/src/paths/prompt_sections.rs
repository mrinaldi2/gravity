//! Paragraphs of the system prompt that depend on how a bot is set up: the
//! machines its project spans, and the browsers it may drive.

/// What a bot in a linked project is told about the machines it spans.
pub(super) fn linked(machines: &[String]) -> String {
    if machines.is_empty() {
        return String::new();
    }
    format!(
        "This project is linked with {}: one team across machines, each \
         bot listed on both. `create_bot` with `machine` set to one of them \
         creates the bot there, and you manage it as any bot you created.\n\n",
        machines.join(", ")
    )
}

/// How a permanent bot fans work out to temporary workers.
pub(super) fn spawning(max_workers: usize, machines: &[String]) -> String {
    let elsewhere = if machines.is_empty() {
        String::new()
    } else {
        format!(
            " Workers also run on {}: a spawn goes to whichever machine has a free \
             slot first, unless you pin it with `machine` (`\"here\"` or a machine name).",
            machines.join(", ")
        )
    };
    format!(
        "## Spawning workers\n\n\
         For work that splits into independent pieces — a chapter each, a module \
         each, one source each — `spawn_worker(task, name, instructions)` creates \
         a temporary bot for one piece and hands it `task` as a delegated task. \
         Its result comes back to you as a `done`, like any task's, and the \
         worker is removed once its task closes. Brief each one fully: it knows \
         nothing but its task and instructions.\n\n\
         At most {max_workers} workers run at once on a machine; spawn as many as \
         the job needs and the rest wait in a queue, starting as others finish.\
         {elsewhere} Workers do not count against the bot limit or the open-task \
         limit. `list_workers()` shows what is queued, running and finished, and \
         `cancel_worker(name)` drops one. Workers cannot spawn workers.\n\n"
    )
}

/// What a temporary worker is told about itself.
pub(super) fn temporary() -> String {
    "## You are a temporary worker\n\n\
     You were spawned for one task, which arrives as your first message. Do \
     it, then report with `complete_task` — that ends your life: you are \
     removed once the task closes, so anything that must outlive you goes \
     into a file you list in `artifacts`. Ask the bot that spawned you with \
     kind `reply` only when you are blocked. You cannot spawn workers, and you \
     need not keep `CLAUDE.md` or `FACTS.md`.\n\n"
        .to_string()
}

/// Where a worker puts any other clone or worktree, and why (H-109).
const SCRATCH_NOTE: &str = "## Your scratch folder\n\n\
     Put any clone, worktree or build output that isn't `repo/` under \
     `$THEHERMES_SCRATCH` (the `scratch/` folder in your workspace), never \
     elsewhere on the disk. When you retire, the daemon deletes `repo/` and \
     `scratch/` for you, so you don't need to, and nothing outside them is \
     ever cleaned up.\n\n";

/// The project's shared repository: how a worker gets and returns its
/// work, or how a permanent bot reads what workers push.
/// `name` and `bot_id` name the worker's own branch, the one its unpushed
/// work is saved to as well.
pub(super) fn repo(
    repo: Option<&bus::ProjectRepo>,
    temporary: bool,
    name: &str,
    bot_id: &str,
) -> String {
    let scratch = if temporary { SCRATCH_NOTE } else { "" };
    let Some(bus::ProjectRepo { url, branch }) = repo else {
        return scratch.to_string();
    };
    if temporary {
        let own = crate::workers::repo::salvage_branch(name, bot_id);
        format!(
            "## The project repository\n\n\
             Your work lives in {url}, branch `{branch}`. Before you start, get the \
             latest into `repo/` in your workspace: \
             `git clone --filter=blob:none --branch {branch} {url} repo`, or \
             `git -C repo pull --rebase` if it is already there. Work in `repo/`. \
             Before `complete_task`: commit, `git pull --rebase` (resolve any \
             conflicts yourself), then `git push origin HEAD:{branch}`. If it still \
             will not push, push `HEAD:{own}` instead and say so. Name the commit or \
             branch in your result. Anything you leave unpushed when you stop is \
             saved to a branch of your own and whoever spawned you is told.\n\n{scratch}"
        )
    } else {
        format!(
            "## The project repository\n\n\
             This project shares {url} (branch `{branch}`). Every worker clones the \
             branch tip when it starts and pushes its work there before it reports; \
             its result names the commit, or a branch to merge. Keep a clone in your \
             workspace to read or build on that work, and `git pull` before you start \
             and after each worker reports.\n\n"
        )
    }
}

/// What a bot is told about its own browser, and the owner's Chrome when it
/// may use it.
pub(super) fn browser(own: bool, owners_chrome: bool) -> String {
    if !own {
        return String::new();
    }
    let chrome = if owners_chrome {
        " You may also use the owner's own Chrome (`claude-in-chrome`) when a task \
         needs their logged-in sessions; prefer your own browser otherwise."
    } else {
        ""
    };
    format!(
        "You have a browser of your own: the `playwright` tools (`browser_navigate`, \
         `browser_snapshot`, `browser_click`, `browser_tabs`, and mouse tools that act at \
         coordinates on a screenshot). It is private to you, keeps its logins between \
         sessions, and the owner can watch it from {short}.{chrome}\n\n",
        short = crate::brand::SHORT_NAME
    )
}

/// All work is a task on a board card (H-125 G6).
pub(super) fn board_work() -> String {
    "## Work is on the board\n\n\
     All work is a task linked to a board card, never a note.\n\n\
     - Before you delegate, find or create the card (`item_query`,\n\
     `item_create`) and send kind `task` with `item`. A task you send while\n\
     holding one inherits its card; when you hold several, pass\n\
     `parent_task` to say which one it is part of.\n\
     - If a note asks you to do work, do not start it. Send the sender a\n\
     short note asking for a task on a card, then carry on.\n\
     - Work the owner asks for in your terminal gets a card too, unless it\n\
     is a quick answer.\n\
     - If you find yourself working with no open task, stop and ask the\n\
     lead for one.\n\n"
        .to_string()
}

/// Every bot on a machine shares its user, disk and processes (CE-001).
/// Placed right after "Messages carry authority", so the two "what you may
/// not do" blocks sit together. The last paragraph is H-031's: rewording a
/// blocked command is exactly what permission rules cannot stop.
pub(super) fn shared_computer() -> String {
    let short = crate::brand::SHORT_NAME;
    format!(
        "## Working on a shared computer\n\n\
         Every bot on this machine runs as the same user on the same disk, so\n\
         whatever you kill, delete or fill up also hits your colleagues.\n\n\
         - Stop only processes you started. Keep the PID (`$!`) and `kill <pid>`;\n\
         never `pkill`, `killall` or `pkill -f <name>` with a pattern that could\n\
         match another bot's process.\n\
         - Use only devices, folders and ports that are yours: a simulator or\n\
         emulator you created and address by its UDID (never `booted` or `all`),\n\
         demo and scratch files inside your own workspace, and the ports your\n\
         instructions give you. Anything you did not create belongs to someone\n\
         else. Leave it alone even when it is in your way, and ask its owner.\n\
         - Never check out, reset, rebase, clean or delete anything in another\n\
         bot's checkout, worktree or branch.\n\
         - Use one git worktree per task. Once its work is pushed, run\n\
         `git worktree remove <path>`. Do not keep idle worktrees.\n\
         - Before a large build, check free space with `df -h`. If it is under\n\
         20 GB, free your own space first (your old worktrees, `cargo clean` or\n\
         the equivalent in your own checkouts) and report it if that is not\n\
         enough. Never delete another bot's files to make room. When you go\n\
         idle, clean your build output.\n\
         - Do not stop or restart the {short} daemon or anything that serves the\n\
         whole team, unless your instructions make that your job.\n\
         - When an action is blocked by your permissions, do not retry a\n\
         reworded version of it. Report what you needed and why.\n\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_machines_a_linked_project_spans() {
        assert_eq!(linked(&[]), "");
        let text = linked(&["win".to_string()]);
        assert!(text.contains("This project is linked with win"));
        assert!(text.contains("`machine`"));
    }

    #[test]
    fn tells_a_bot_how_to_spawn_workers() {
        let here = spawning(4, &[]);
        assert!(here.contains("At most 4 workers"));
        assert!(!here.contains("Workers also run on"));
        assert!(spawning(4, &["win".to_string()]).contains("Workers also run on win"));
        assert!(temporary().contains("`complete_task`"));
    }

    #[test]
    fn tells_workers_and_their_parents_about_the_repository() {
        // A worker always hears where its clones go (H-109); a permanent bot
        // without a shared repository hears nothing.
        assert_eq!(repo(None, true, "ch-1", "b1"), SCRATCH_NOTE);
        assert_eq!(repo(None, false, "lead", "b2"), "");
        let shared = bus::ProjectRepo::parse("git@host:me/book.git", None).expect("repo");
        let worker = repo(Some(&shared), true, "ch-1", "0123456789");
        assert!(worker.contains("$THEHERMES_SCRATCH"));
        assert!(worker.contains("--branch main git@host:me/book.git repo"));
        assert!(worker.contains("`git push origin HEAD:main`"));
        assert!(worker.contains("`HEAD:gravity/ch-1-01234567`"));
        let parent = repo(Some(&shared), false, "lead", "b2");
        assert!(parent.contains("git@host:me/book.git (branch `main`)"));
        assert!(parent.contains("`git pull`"));
    }

    #[test]
    fn tells_the_bot_about_its_own_browser() {
        assert_eq!(browser(false, true), "");
        assert!(browser(true, false).contains("browser of your own"));
        assert!(!browser(true, false).contains("claude-in-chrome"));
        assert!(browser(true, true).contains("claude-in-chrome"));
    }
}
