//! Diagnostics printed once per distinct message.

use std::cell::RefCell;
use std::collections::HashSet;

thread_local! {
    static SEEN: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

pub fn warn_once(msg: String) {
    SEEN.with(|s| {
        if s.borrow_mut().insert(msg.clone()) {
            eprintln!("dviavi: warning: {msg}");
        }
    });
}
