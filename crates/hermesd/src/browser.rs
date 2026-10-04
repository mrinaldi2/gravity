//! A browser of each bot's own. Every bot's session gets the Playwright MCP
//! server driving a Chrome with a profile in the bot's directory, headless by
//! default, so bots never open tabs in the owner's Chrome. The daemon finds a
//! running browser through its DevToolsActivePort file, streams it to the
//! app, and passes the owner's mouse and keyboard back to it. See "Browser" in `docs/protocol.md`.

mod cdp;
pub mod input;
pub mod setup;
pub mod streams;
pub mod view;

pub use setup::{BotBrowser, BrowserConfig};
