//! Layers: objects on a layer paint above those on the layers below it, and a layer can
//! be hidden or locked as a whole.

use std::sync::Arc;

/// A layer of the board.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub id: Arc<str>,
    pub name: Arc<str>,
    /// Layers paint from the lowest order up.
    pub order: f64,
    pub visible: bool,
    pub locked: bool,
}

impl Layer {
    /// The layer of objects that name none (or one that no longer exists). It is always
    /// there, even on boards that never stored it.
    pub const BASE: &str = "base";

    pub fn base() -> Self {
        Self {
            id: Self::BASE.into(),
            name: "Layer 1".into(),
            order: 0.0,
            visible: true,
            locked: false,
        }
    }

    pub fn is_base(&self) -> bool {
        &*self.id == Self::BASE
    }

    /// How it is stored: `order;flags;name`, with `h` in the flags for hidden and `l` for
    /// locked.
    pub(crate) fn to_value(&self) -> String {
        let mut flags = String::new();
        if !self.visible {
            flags.push('h');
        }
        if self.locked {
            flags.push('l');
        }
        format!("{};{flags};{}", self.order, self.name)
    }

    pub(crate) fn parse(id: &str, value: &str) -> Option<Self> {
        let mut parts = value.splitn(3, ';');
        let order = parts.next()?.parse().ok()?;
        let flags = parts.next()?;
        let name = parts.next()?;
        Some(Self {
            id: id.into(),
            name: name.into(),
            order,
            visible: !flags.contains('h'),
            locked: flags.contains('l'),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_its_stored_value() {
        let layer = Layer {
            id: "a1".into(),
            name: "Notes; drafts".into(),
            order: 2.5,
            visible: false,
            locked: true,
        };
        assert_eq!(Layer::parse("a1", &layer.to_value()), Some(layer));
        assert_eq!(Layer::parse("x", "junk"), None);
    }
}
