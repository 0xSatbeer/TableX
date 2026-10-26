//! A console's own session and its transaction: what a run reuses, whether it must BEGIN, and
//! what closing rolls back. Plain data, so its rules test without an editor.

use crate::db::Session;

#[derive(Default)]
pub struct ConsoleSession {
    /// Opened on the first run and kept, so each console has its own.
    session: Option<Session>,
    /// Manual mode: runs open a transaction that only Commit or Rollback ends.
    pub(super) manual: bool,
    pub in_transaction: bool,
}

/// What a run starts from.
pub(super) struct RunStart {
    /// The session to reuse; None connects a new one.
    pub(super) session: Option<Session>,
    /// Manual mode outside a transaction: BEGIN first.
    pub(super) begin: bool,
    pub(super) autocommit: bool,
    /// The session was found dropped with a transaction open, which went with it.
    pub(super) rolled_back: bool,
}

impl ConsoleSession {
    /// The session, unless the server closed it.
    pub(super) fn open(&self) -> Option<Session> {
        self.session.clone().filter(|s| !s.is_closed())
    }

    /// What the status bar shows for this console.
    pub(super) fn mode(&self) -> &'static str {
        match (self.manual, self.in_transaction) {
            (_, true) => "TX",
            (true, false) => "MANUAL",
            (false, false) => "AUTO-COMMIT",
        }
    }

    pub(super) fn start_run(&mut self) -> RunStart {
        let session = self.open();
        // A dropped session took its transaction with it; a new one must BEGIN again.
        let rolled_back = session.is_none() && std::mem::take(&mut self.in_transaction);
        RunStart {
            session,
            begin: self.manual && !self.in_transaction,
            autocommit: !self.manual,
            rolled_back,
        }
    }

    /// The session a run connected or reused is the console's from now on.
    pub(super) fn ran_on(&mut self, session: Session) {
        self.session = Some(session);
    }

    /// Keeps `session` unless a run has kept one meanwhile, as listing the context does.
    pub(super) fn keep(&mut self, session: Session) {
        if self.open().is_none() {
            self.session = Some(session);
        }
    }

    pub(super) fn began(&mut self) {
        self.in_transaction = true;
    }

    pub(super) fn ended(&mut self) {
        self.in_transaction = false;
    }

    /// The connection was lost: the next run connects again.
    pub(super) fn lost(&mut self) {
        (self.session, self.in_transaction) = (None, false);
    }

    /// Lets the session go: it and whether a transaction was open on it, to roll back.
    pub(super) fn take(&mut self) -> Option<(Session, bool)> {
        let in_transaction = std::mem::take(&mut self.in_transaction);
        self.session.take().map(|s| (s, in_transaction))
    }

    /// The session Commit or Rollback ends the open transaction on.
    pub(super) fn transaction(&self) -> Option<Session> {
        self.session.clone().filter(|_| self.in_transaction)
    }

    /// Switches mode. Leaving manual with a transaction open changes nothing and returns false:
    /// the transaction must end first.
    pub(super) fn set_manual(&mut self, manual: bool) -> bool {
        if !manual && self.in_transaction {
            return false;
        }
        self.manual = manual;
        true
    }

    /// A schema pick SETs `search_path` only outside a transaction, whose rollback would undo it.
    pub(super) fn can_set_schema(&self) -> bool {
        !self.in_transaction
    }
}
