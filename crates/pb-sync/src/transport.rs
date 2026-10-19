//! The wire between a board and the server.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::message::Message;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Open,
    /// Gone, and why.
    Closed(String),
}

/// Sends and receives whole messages without blocking (it must work in the browser).
pub trait Transport {
    fn send(&mut self, message: &Message);
    /// The next message that arrived, if any.
    fn try_recv(&mut self) -> Option<Message>;
    fn status(&self) -> Status;
}

/// No connection: the board is not shared.
pub struct LocalOnly;

impl Transport for LocalOnly {
    fn send(&mut self, _message: &Message) {}

    fn try_recv(&mut self) -> Option<Message> {
        None
    }

    fn status(&self) -> Status {
        Status::Closed("not shared".into())
    }
}

/// Messages waiting in one direction of a [`Memory`] link.
type Queue = Rc<RefCell<VecDeque<Vec<u8>>>>;

/// Both ends of a connection in memory, for tests: what one end sends, the other
/// receives. Messages go through their binary form, as on the wire.
pub struct Memory {
    inbox: Queue,
    outbox: Queue,
    open: Rc<RefCell<bool>>,
}

impl Memory {
    pub fn pair() -> (Self, Self) {
        let (a, b) = (Queue::default(), Queue::default());
        let open = Rc::new(RefCell::new(true));
        (
            Self {
                inbox: a.clone(),
                outbox: b.clone(),
                open: open.clone(),
            },
            Self {
                inbox: b,
                outbox: a,
                open,
            },
        )
    }

    pub fn close(&self) {
        *self.open.borrow_mut() = false;
    }
}

impl Transport for Memory {
    fn send(&mut self, message: &Message) {
        if *self.open.borrow() {
            self.outbox.borrow_mut().push_back(message.encode());
        }
    }

    fn try_recv(&mut self) -> Option<Message> {
        let bytes = self.inbox.borrow_mut().pop_front()?;
        Some(Message::decode(&bytes).expect("only whole messages are sent"))
    }

    fn status(&self) -> Status {
        if *self.open.borrow() {
            Status::Open
        } else {
            Status::Closed("closed".into())
        }
    }
}
