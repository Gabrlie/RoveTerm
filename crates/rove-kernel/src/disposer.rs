//! Revocation handles for registrations.

use std::fmt;

/// Revokes one registration when dropped or when [`Disposer::dispose`] is called.
///
/// Every registration on a [`Context`](crate::Context) returns a `Disposer`. Dropping it revokes the
/// registration exactly once; a disposer whose context has already been dropped does nothing.
#[must_use = "dropping a Disposer immediately revokes its registration"]
pub struct Disposer {
    revoke: Option<Box<dyn FnOnce() + Send>>,
}

impl Disposer {
    /// Creates a disposer that runs `revoke` once when dropped.
    pub fn new(revoke: impl FnOnce() + Send + 'static) -> Self {
        Self {
            revoke: Some(Box::new(revoke)),
        }
    }

    /// Creates a disposer that revokes nothing.
    pub fn noop() -> Self {
        Self { revoke: None }
    }

    /// Revokes the registration now. Equivalent to dropping the disposer.
    pub fn dispose(self) {
        drop(self);
    }
}

impl Drop for Disposer {
    fn drop(&mut self) {
        if let Some(revoke) = self.revoke.take() {
            revoke();
        }
    }
}

impl fmt::Debug for Disposer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Disposer")
            .field("armed", &self.revoke.is_some())
            .finish()
    }
}

/// An ordered group of disposers that revokes in reverse registration order when dropped.
#[derive(Debug, Default)]
#[must_use = "dropping Disposers immediately revokes every registration it holds"]
pub struct Disposers(Vec<Disposer>);

impl Disposers {
    /// Creates an empty group.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Appends a disposer; it is revoked before every disposer pushed earlier.
    pub fn push(&mut self, disposer: Disposer) {
        self.0.push(disposer);
    }

    /// Returns the number of held disposers.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` when the group holds no disposers.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<Disposer>> for Disposers {
    fn from(disposers: Vec<Disposer>) -> Self {
        Self(disposers)
    }
}

impl Extend<Disposer> for Disposers {
    fn extend<T: IntoIterator<Item = Disposer>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl Drop for Disposers {
    fn drop(&mut self) {
        while let Some(disposer) = self.0.pop() {
            drop(disposer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn disposer_runs_once_on_drop() {
        let count = Arc::new(Mutex::new(0));
        let counter = count.clone();
        let disposer = Disposer::new(move || *counter.lock().unwrap() += 1);
        disposer.dispose();
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[test]
    fn disposers_revoke_in_reverse_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut group = Disposers::new();
        for i in 0..3 {
            let order = order.clone();
            group.push(Disposer::new(move || order.lock().unwrap().push(i)));
        }
        drop(group);
        assert_eq!(*order.lock().unwrap(), vec![2, 1, 0]);
    }
}
