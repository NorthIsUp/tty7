//! The fork's reads of a pane's link, kept out of `remote.rs` so a rebase on
//! upstream touches as little of it as possible.

use super::RemoteTerminal;
use crate::terminal::TermSize;

impl RemoteTerminal {
    /// The grid and device-pixel cell this pane last asked for, once a
    /// layout has sized it; `None` while it still runs at its spawn size.
    pub(crate) fn laid_out_grid(&self) -> Option<(TermSize, u16, u16)> {
        let (w, h) = self.synced_cell;
        self.synced_size.then_some((self.size, w, h))
    }
}
