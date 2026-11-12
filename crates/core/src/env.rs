//! An injectable environment. The TypeScript packages read `process.env` at
//! call time (and tests stub it with `vi.stubEnv`); the Rust ports take an
//! [`Env`] so tests pass a plain map and never touch the process
//! environment (which is not thread safe to change).

use std::collections::{BTreeMap, HashMap};

/// A source of environment variables.
pub trait Env {
    /// The value of `name`, or `None` when it is not set.
    fn var(&self, name: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// No variable set.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyEnv;

impl Env for EmptyEnv {
    fn var(&self, _name: &str) -> Option<String> {
        None
    }
}

impl Env for [(&str, &str)] {
    fn var(&self, name: &str) -> Option<String> {
        self.iter()
            .rev()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    }
}

impl<const N: usize> Env for [(&str, &str); N] {
    fn var(&self, name: &str) -> Option<String> {
        self.as_slice().var(name)
    }
}

impl Env for Vec<(String, String)> {
    fn var(&self, name: &str) -> Option<String> {
        self.iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }
}

impl Env for BTreeMap<String, String> {
    fn var(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

impl Env for HashMap<String, String> {
    fn var(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

impl<T: Env + ?Sized> Env for &T {
    fn var(&self, name: &str) -> Option<String> {
        (**self).var(name)
    }
}
