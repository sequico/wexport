//! The one pipeline behind two phones: what a chat archive asks of a data source.
//!
//! Both adapters produce the same `Chat`/`ChatStat`/`Message`, so the archive, the viewer and the
//! reporter never learn which phone a conversation came from.

use crate::error::Result;
use crate::msgstore::{Chat, ChatStat, Message, Msgstore};

pub trait Store {
    fn chat_stats(&self) -> Result<Vec<ChatStat>>;

    /// Every message of one chat, in time order, handed to `sink` one at a time.
    fn each_message(&self, chat: &Chat, sink: &mut dyn FnMut(Message) -> Result<()>) -> Result<()>;

    /// Media files the source can hand over, and media the database references: `(present, named)`.
    fn media_counts(&self) -> Result<(i64, i64)>;

    /// The database's own verdict on its pages.
    fn integrity_check(&self) -> Result<()>;

    /// What `-d` says about the schema this build recognised.
    fn diagnostics(&self) -> Vec<String>;
}

impl Store for Msgstore {
    fn chat_stats(&self) -> Result<Vec<ChatStat>> {
        Msgstore::chat_stats(self)
    }

    fn each_message(&self, chat: &Chat, sink: &mut dyn FnMut(Message) -> Result<()>) -> Result<()> {
        Msgstore::each_message(self, chat, sink)
    }

    fn media_counts(&self) -> Result<(i64, i64)> {
        Ok(Msgstore::media_counts(self))
    }

    fn integrity_check(&self) -> Result<()> {
        Msgstore::integrity_check(self)
    }

    fn diagnostics(&self) -> Vec<String> {
        Msgstore::diagnostics(self)
    }
}
